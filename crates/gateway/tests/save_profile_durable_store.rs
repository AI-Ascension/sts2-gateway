// SPDX-License-Identifier: MIT

//! Durable-store recovery for isolated user-data provisioning (gateway issue #51).
//!
//! These tests use the SQLite-backed record store so an intent survives a real process-level
//! reopen (a fresh connection to the same file), not just a reopened in-memory map.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_gateway::{
    InMemoryLaunchProfileBindingPort, InMemoryUserDataPort, LAUNCH_PROFILE_CONTRACT,
    LAUNCH_PROFILE_ID, LaunchProfileBinding, SaveProfileContext, SqliteUserDataRecordStore,
    UserDataCreateOutcome, UserDataDescriptor, UserDataIdentity, UserDataInspection,
    UserDataProvenance, UserDataProvisioner, UserDataProvisioningError, UserDataProvisioningRecord,
    UserDataProvisioningStatus, UserDataRecordStore,
};

const INSTANCE: &str = "instance-1";

fn context(correlation: &str) -> SaveProfileContext {
    SaveProfileContext {
        instance_id: INSTANCE.to_owned(),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        correlation_id: correlation.to_owned(),
    }
}

fn descriptor(identity: u64, operation_id: &str) -> UserDataDescriptor {
    UserDataDescriptor {
        identity: UserDataIdentity::new(identity),
        provenance: UserDataProvenance {
            owner: String::from("gateway"),
            instance_id: INSTANCE.to_owned(),
            operation_id: operation_id.to_owned(),
            contract: LAUNCH_PROFILE_CONTRACT.to_owned(),
        },
        baseline: None,
    }
}

fn binding(identity: u64) -> Result<LaunchProfileBinding, String> {
    LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, UserDataIdentity::new(identity))
        .map_err(|error| format!("{error:?}"))
}

fn record(identity: u64, operation_id: &str) -> Result<UserDataProvisioningRecord, String> {
    Ok(UserDataProvisioningRecord {
        operation_id: operation_id.to_owned(),
        context: context("durable"),
        descriptor: descriptor(identity, operation_id),
        launch_profile: binding(identity)?,
        status: UserDataProvisioningStatus::Pending,
        guidance: None,
    })
}

fn temp_database() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "sts2-gateway-save-profile-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn remove_database(path: &Path) {
    for suffix in ["", "-wal", "-shm", ".save-profile.lock"] {
        let candidate = format!("{}{suffix}", path.display());
        let _ = std::fs::remove_file(candidate);
    }
}

#[test]
fn durable_store_survives_reopen_and_recovers_the_identity() -> Result<(), String> {
    let path = temp_database();
    let context = context("durable-recovery");

    // A lost reply after a possible host write leaves an Unknown intent on disk.
    {
        let mut port = InMemoryUserDataPort::default();
        port.set_outcome(
            UserDataIdentity::new(1),
            UserDataCreateOutcome::TimeoutAfterWrite,
        );
        let store = SqliteUserDataRecordStore::open(&path).map_err(|error| format!("{error:?}"))?;
        let mut provisioner =
            UserDataProvisioner::new(4, port, store, InMemoryLaunchProfileBindingPort::default())
                .map_err(|error| format!("{error:?}"))?;
        let outcome = provisioner
            .create_disposable(context.clone(), "op-1", LAUNCH_PROFILE_ID)
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(outcome.status, UserDataProvisioningStatus::Unknown);
    }

    // A fresh connection to the same file is a new process view of the durable journal.
    {
        let mut port = InMemoryUserDataPort::default();
        port.set_inspection(
            UserDataIdentity::new(1),
            UserDataInspection::Owned(descriptor(1, "op-1").provenance),
        );
        let store = SqliteUserDataRecordStore::open(&path).map_err(|error| format!("{error:?}"))?;
        let mut provisioner =
            UserDataProvisioner::new(4, port, store, InMemoryLaunchProfileBindingPort::default())
                .map_err(|error| format!("{error:?}"))?;
        let outcome = provisioner
            .reconcile(&context, "op-1")
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(outcome.status, UserDataProvisioningStatus::Created);
        assert_eq!(
            outcome.descriptor.as_ref().map(|value| value.identity),
            Some(UserDataIdentity::new(1)),
            "the durable identity must be recovered, not reallocated"
        );
        assert!(
            provisioner.port().created.is_empty(),
            "reconcile must not repeat a possibly-accepted create"
        );
    }

    remove_database(&path);
    Ok(())
}

#[test]
fn durable_store_refuses_duplicate_and_missing_records() -> Result<(), String> {
    let mut store =
        SqliteUserDataRecordStore::open_in_memory().map_err(|error| format!("{error:?}"))?;
    store
        .insert(record(1, "op-1")?)
        .map_err(|error| format!("{error:?}"))?;
    match store.insert(record(1, "op-1")?) {
        Err(UserDataProvisioningError::OperationConflict) => {}
        other => return Err(format!("expected duplicate refusal, got {other:?}")),
    }
    match store.update(record(1, "op-missing")?) {
        Err(UserDataProvisioningError::OperationNotFound) => {}
        other => return Err(format!("expected operation-not-found, got {other:?}")),
    }
    let listed = store.list().map_err(|error| format!("{error:?}"))?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].operation_id, "op-1");
    Ok(())
}

#[test]
fn durable_store_refuses_a_competing_owner() -> Result<(), String> {
    let path = temp_database();
    let first = SqliteUserDataRecordStore::open(&path).map_err(|error| format!("{error:?}"))?;
    match SqliteUserDataRecordStore::open(&path) {
        Err(UserDataProvisioningError::OperationConflict) => {}
        Err(other) => return Err(format!("expected competing-owner refusal, got {other:?}")),
        Ok(_) => return Err(String::from("competing owner was not refused")),
    }
    drop(first);
    SqliteUserDataRecordStore::open(&path)
        .map_err(|error| format!("reopen after owner drop failed: {error:?}"))?;
    remove_database(&path);
    Ok(())
}

#[test]
fn durable_store_rejects_sqlite_uri_aliases() -> Result<(), String> {
    let path = temp_database();
    let uri = format!("file:{}", path.display());
    match SqliteUserDataRecordStore::open(uri.as_str()) {
        Err(UserDataProvisioningError::PersistenceFailed) => {}
        Err(other) => return Err(format!("expected uri rejection, got {other:?}")),
        Ok(_) => return Err(String::from("sqlite uri alias was not rejected")),
    }
    Ok(())
}

#[test]
fn durable_store_rejects_an_empty_path() -> Result<(), String> {
    match SqliteUserDataRecordStore::open("") {
        Err(UserDataProvisioningError::PersistenceFailed) => {}
        Err(other) => return Err(format!("expected empty-path rejection, got {other:?}")),
        Ok(_) => return Err(String::from("empty path was not rejected")),
    }
    Ok(())
}
