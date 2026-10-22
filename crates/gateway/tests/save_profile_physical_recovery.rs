// SPDX-License-Identifier: MIT

//! Real filesystem and SQLite recovery; no native game or accepted launch binding is implied.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sts2_gateway::{
    FilesystemUserDataPort, InMemoryLaunchProfileBindingPort, LAUNCH_PROFILE_ID,
    SaveProfileContext, SqliteUserDataRecordStore, UserDataCreateOutcome, UserDataCreateRequest,
    UserDataIdentity, UserDataInspection, UserDataPort, UserDataPortError, UserDataProvisioner,
    UserDataProvisioningError, UserDataProvisioningRecord, UserDataProvisioningStatus,
    UserDataRecordStore,
};
use uuid::Uuid;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("sts2-allocation-recovery-{}", Uuid::new_v4()));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        fs::create_dir(path.join("data")).map_err(|error| error.to_string())?;
        fs::write(path.join("existing-profile"), b"preserve").map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    fn store(&self) -> Result<SqliteUserDataRecordStore, String> {
        SqliteUserDataRecordStore::open(self.0.join("allocations.sqlite"))
            .map_err(|error| format!("{error:?}"))
    }

    fn port(&self, calls: Arc<AtomicUsize>, lose_reply: bool) -> Result<RecordingPort, String> {
        Ok(RecordingPort {
            inner: FilesystemUserDataPort::try_new(self.0.join("data"), 4)
                .map_err(|error| format!("{error:?}"))?,
            calls,
            lose_reply,
        })
    }

    fn assert_preserved(&self) -> Result<(), String> {
        assert_eq!(
            fs::read(self.0.join("existing-profile")).map_err(|error| error.to_string())?,
            b"preserve"
        );
        assert!(!self.0.join("data/run-2").exists());
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RecordingPort {
    inner: FilesystemUserDataPort,
    calls: Arc<AtomicUsize>,
    lose_reply: bool,
}

impl UserDataPort for RecordingPort {
    fn inspect(
        &mut self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError> {
        self.inner.inspect(identity)
    }

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.inner.create(request)?;
        Ok(if self.lose_reply {
            UserDataCreateOutcome::Unknown
        } else {
            result
        })
    }
}

struct FailResultStore(SqliteUserDataRecordStore);

impl UserDataRecordStore for FailResultStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        self.0.list()
    }
    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        self.0.insert(record)
    }
    fn update(&mut self, _: UserDataProvisioningRecord) -> Result<(), UserDataProvisioningError> {
        Err(UserDataProvisioningError::PersistenceFailed)
    }
}

fn context() -> SaveProfileContext {
    SaveProfileContext {
        instance_id: "instance-1".to_owned(),
        caller_id: "caller-1".to_owned(),
        session_id: "session-1".to_owned(),
        lease_id: "lease-1".to_owned(),
        lease_epoch: 1,
        correlation_id: "correlation-1".to_owned(),
    }
}

#[test]
fn physical_create_with_lost_reply_reopens_and_replays_the_same_identity() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut first = UserDataProvisioner::new(
        4,
        fixture.port(calls.clone(), true)?,
        fixture.store()?,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let result = first
        .create_disposable(context(), "op-create", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result.status, UserDataProvisioningStatus::Unknown);
    drop(first);
    let mut reopened = UserDataProvisioner::new(
        4,
        fixture.port(calls.clone(), false)?,
        fixture.store()?,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    for result in [
        reopened.reconcile(&context(), "op-create"),
        reopened.create_disposable(context(), "op-create", LAUNCH_PROFILE_ID),
    ] {
        let result = result.map_err(|error| format!("{error:?}"))?;
        assert_eq!(result.status, UserDataProvisioningStatus::Created);
        assert_eq!(
            result.descriptor.map(|value| value.identity),
            Some(UserDataIdentity::new(1))
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fixture.assert_preserved()
}

#[test]
fn physical_write_survives_failed_result_persistence_without_recreating() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut first = UserDataProvisioner::new(
        4,
        fixture.port(calls.clone(), false)?,
        FailResultStore(fixture.store()?),
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        first.create_disposable(context(), "op-create", LAUNCH_PROFILE_ID),
        Err(UserDataProvisioningError::PersistenceFailed)
    );
    drop(first);
    let mut reopened = UserDataProvisioner::new(
        4,
        fixture.port(calls.clone(), false)?,
        fixture.store()?,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let result = reopened
        .reconcile(&context(), "op-create")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result.status, UserDataProvisioningStatus::Created);
    assert_eq!(
        result.descriptor.map(|value| value.identity),
        Some(UserDataIdentity::new(1))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fixture.assert_preserved()
}

#[test]
fn a_stale_created_receipt_cannot_recreate_or_adopt_missing_or_partial_contents()
-> Result<(), String> {
    for partial in [false, true] {
        let fixture = Fixture::new()?;
        let calls = Arc::new(AtomicUsize::new(0));
        let mut first = UserDataProvisioner::new(
            4,
            fixture.port(calls.clone(), false)?,
            fixture.store()?,
            InMemoryLaunchProfileBindingPort::default(),
        )
        .map_err(|error| format!("{error:?}"))?;
        first
            .create_disposable(context(), "op-create", LAUNCH_PROFILE_ID)
            .map_err(|error| format!("{error:?}"))?;
        drop(first);
        let path = fixture.0.join("data/run-1");
        fs::remove_file(path.join("gateway-provenance.json")).map_err(|error| error.to_string())?;
        if partial {
            fs::write(path.join("unknown-save"), b"preserve partial")
                .map_err(|error| error.to_string())?;
        } else {
            fs::remove_dir(&path).map_err(|error| error.to_string())?;
        }
        let mut reopened = UserDataProvisioner::new(
            4,
            fixture.port(calls.clone(), false)?,
            fixture.store()?,
            InMemoryLaunchProfileBindingPort::default(),
        )
        .map_err(|error| format!("{error:?}"))?;
        for _ in 0..2 {
            let result = reopened
                .create_disposable(context(), "op-create", LAUNCH_PROFILE_ID)
                .map_err(|error| format!("{error:?}"))?;
            assert_eq!(
                result.status,
                if partial {
                    UserDataProvisioningStatus::Blocked
                } else {
                    UserDataProvisioningStatus::Unknown
                }
            );
            assert!(result.descriptor.is_none());
        }
        if partial {
            assert_eq!(
                fs::read(path.join("unknown-save")).map_err(|error| error.to_string())?,
                b"preserve partial"
            );
        } else {
            assert!(!path.exists());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        fixture.assert_preserved()?;
    }
    Ok(())
}

#[test]
fn temporarily_missing_root_recovers_the_original_identity_without_another_create()
-> Result<(), String> {
    let fixture = Fixture::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut provisioner = UserDataProvisioner::new(
        4,
        fixture.port(calls.clone(), false)?,
        fixture.store()?,
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    provisioner
        .create_disposable(context(), "op-create", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    let root = fixture.0.join("data");
    let retained = fixture.0.join("retained-data");
    fs::rename(&root, &retained).map_err(|error| error.to_string())?;
    let result = provisioner
        .reconcile(&context(), "op-create")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result.status, UserDataProvisioningStatus::Unknown);
    assert!(result.descriptor.is_none());
    assert!(!root.exists());
    fs::rename(&retained, &root).map_err(|error| error.to_string())?;
    let recovered = provisioner
        .reconcile(&context(), "op-create")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(recovered.status, UserDataProvisioningStatus::Created);
    assert_eq!(
        recovered.descriptor.map(|value| value.identity),
        Some(UserDataIdentity::new(1))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fixture.assert_preserved()
}
