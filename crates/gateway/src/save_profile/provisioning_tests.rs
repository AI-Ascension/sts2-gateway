// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::ledger_tests::context;
use super::*;

/// Shared allocation store. A clone reopens the same records, which is how these deterministic
/// tests model a gateway restart over an injected durable store.
#[derive(Clone, Default)]
struct SharedProvisioningStore {
    records: Arc<Mutex<BTreeMap<(String, String), UserDataProvisioningRecord>>>,
}

impl UserDataRecordStore for SharedProvisioningStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        let records = self
            .records
            .lock()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Ok(records.values().cloned().collect())
    }

    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        let key = (
            record.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if records.contains_key(&key) {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        records.insert(key, record);
        Ok(())
    }

    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        let key = (
            record.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if !records.contains_key(&key) {
            return Err(UserDataProvisioningError::OperationNotFound);
        }
        records.insert(key, record);
        Ok(())
    }
}

/// Launch-profile adapter that records every binding request and can refuse on demand.
#[derive(Clone, Default)]
struct RecordingBindingPort {
    calls: Arc<Mutex<Vec<UserDataIdentity>>>,
    refuse: bool,
}

impl LaunchProfileBindingPort for RecordingBindingPort {
    fn bind_disposable(
        &mut self,
        user_data: UserDataIdentity,
    ) -> Result<LaunchProfileBinding, LaunchProfileBindingError> {
        self.calls
            .lock()
            .map_err(|_| LaunchProfileBindingError::Contract)?
            .push(user_data);
        if self.refuse {
            return Err(LaunchProfileBindingError::Profile);
        }
        LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, user_data)
    }
}

#[test]
fn allocation_port_errors_fail_closed_without_overwrite_or_adoption() -> Result<(), String> {
    for (error, expected) in [
        (
            UserDataPortError::ExistingContents,
            UserDataProvisioningStatus::Blocked,
        ),
        (
            UserDataPortError::UnknownContents,
            UserDataProvisioningStatus::Blocked,
        ),
        (
            UserDataPortError::Unavailable,
            UserDataProvisioningStatus::Rejected,
        ),
        (
            UserDataPortError::TimeoutBeforeWrite,
            UserDataProvisioningStatus::Unknown,
        ),
    ] {
        let mut port = InMemoryUserDataPort::default();
        port.set_create_error(UserDataIdentity::new(1), error);
        let mut provisioner = UserDataProvisioner::new(
            4,
            port,
            InMemoryUserDataRecordStore::default(),
            RecordingBindingPort::default(),
        )
        .map_err(|error| format!("{error:?}"))?;
        let outcome = provisioner
            .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(outcome.status, expected);
        assert!(outcome.descriptor.is_none());
        assert_eq!(provisioner.port().created.len(), 1);
        let retry = provisioner
            .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(retry.status, expected);
        assert_eq!(
            provisioner.port().created.len(),
            1,
            "a refused allocation must never be retried against the same root"
        );
    }
    Ok(())
}

fn assert_blocked_without_create(port: InMemoryUserDataPort) -> Result<(), String> {
    let mut provisioner = UserDataProvisioner::new(
        4,
        port,
        InMemoryUserDataRecordStore::default(),
        RecordingBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let outcome = provisioner
        .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(outcome.status, UserDataProvisioningStatus::Blocked);
    assert!(outcome.guidance.is_some());
    assert!(provisioner.port().created.is_empty());
    Ok(())
}

#[test]
fn traversal_inspection_blocks_before_any_create_call() -> Result<(), String> {
    for inspection in [
        UserDataInspection::Traversal,
        UserDataInspection::SymlinkEscape,
    ] {
        let mut port = InMemoryUserDataPort::default();
        port.set_inspection(UserDataIdentity::new(1), inspection);
        assert_blocked_without_create(port)?;
    }
    for error in [
        UserDataPortError::Traversal,
        UserDataPortError::SymlinkEscape,
        UserDataPortError::UnknownContents,
    ] {
        let mut port = InMemoryUserDataPort::default();
        port.set_inspect_error(UserDataIdentity::new(1), error);
        assert_blocked_without_create(port)?;
    }
    Ok(())
}

#[test]
fn launch_profile_binding_is_injected_and_its_absence_blocks_allocation() -> Result<(), String> {
    let binding = RecordingBindingPort::default();
    let calls = Arc::clone(&binding.calls);
    let mut provisioner = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        InMemoryUserDataRecordStore::default(),
        binding,
    )
    .map_err(|error| format!("{error:?}"))?;
    let created = provisioner
        .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("descriptor missing"))?;
    let record = provisioner
        .operation("instance-1", "op-1")
        .ok_or_else(|| String::from("record missing"))?;
    assert_eq!(record.launch_profile.contract, LAUNCH_PROFILE_CONTRACT);
    assert_eq!(record.launch_profile.profile_id, LAUNCH_PROFILE_ID);
    assert_eq!(record.launch_profile.user_data, created.identity);
    assert_eq!(
        calls
            .lock()
            .map_err(|_| String::from("call log poisoned"))?
            .as_slice(),
        [created.identity]
    );

    let mut refusing = InMemoryLaunchProfileBindingPort::default();
    refusing.refuse(UserDataIdentity::new(1));
    let mut provisioner = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        InMemoryUserDataRecordStore::default(),
        refusing,
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        provisioner.create_disposable(context("corr-2"), "op-2", LAUNCH_PROFILE_ID),
        Err(UserDataProvisioningError::Port(
            UserDataPortError::Unavailable
        ))
    );
    assert!(provisioner.port().created.is_empty());
    Ok(())
}

#[test]
fn reopened_allocation_store_keeps_identities_and_does_not_readopt() -> Result<(), String> {
    let store = SharedProvisioningStore::default();
    let mut first = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        store.clone(),
        RecordingBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let created = first
        .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("descriptor missing"))?;
    assert_eq!(created.identity, UserDataIdentity::new(1));

    let mut reopened = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        store,
        RecordingBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let next = reopened
        .create_disposable(context("corr-2"), "op-2", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("descriptor missing"))?;
    assert_eq!(next.identity, UserDataIdentity::new(2));
    let replayed = reopened
        .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("descriptor missing"))?;
    assert_eq!(replayed.identity, created.identity);
    assert_eq!(reopened.port().created.len(), 1);
    Ok(())
}
