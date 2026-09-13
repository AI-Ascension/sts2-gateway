// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::guidance::RecoveryGuidance;
use super::provisioning_types::{
    UserDataCreateOutcome, UserDataCreateRequest, UserDataInspection, UserDataPort,
    UserDataPortError, UserDataProvisioningError, UserDataProvisioningOutcome,
    UserDataProvisioningRecord, UserDataProvisioningStatus, UserDataRecordStore,
};
use super::provisioning_validation::{
    blocked, blocked_record, descriptor, guidance, outcome, unknown_guidance, update_record,
    validate_operation_id, validate_request,
};
use super::types::{
    LAUNCH_PROFILE_CONTRACT, LaunchProfileBinding, SaveProfileContext, UserDataDescriptor,
    UserDataIdentity,
};

const MAX_ALLOCATIONS: usize = 64;

/// Allocates a fresh, gateway-owned user-data identity and records every attempt.
pub struct UserDataProvisioner<P, S> {
    capacity: usize,
    next_identity: u64,
    port: P,
    store: S,
    records: BTreeMap<(String, String), UserDataProvisioningRecord>,
}

impl<P: UserDataPort, S: UserDataRecordStore> UserDataProvisioner<P, S> {
    pub fn new(capacity: usize, port: P, mut store: S) -> Result<Self, UserDataProvisioningError> {
        if capacity == 0 || capacity > MAX_ALLOCATIONS {
            return Err(UserDataProvisioningError::CapacityExceeded);
        }
        let persisted = store.list()?;
        let next_identity = persisted
            .iter()
            .map(|record| record.descriptor.identity.value())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(UserDataProvisioningError::CapacityExceeded)?;
        let records = persisted
            .into_iter()
            .map(|record| {
                (
                    (
                        record.context.instance_id.clone(),
                        record.operation_id.clone(),
                    ),
                    record,
                )
            })
            .collect();
        Ok(Self {
            capacity,
            next_identity,
            port,
            store,
            records,
        })
    }

    pub fn create_disposable(
        &mut self,
        context: SaveProfileContext,
        operation_id: impl Into<String>,
        profile_id: impl Into<String>,
    ) -> Result<UserDataProvisioningOutcome, UserDataProvisioningError> {
        let operation_id = operation_id.into();
        let profile_id = profile_id.into();
        validate_request(&context, &operation_id, &profile_id)?;
        let key = (context.instance_id.clone(), operation_id.clone());
        if let Some(existing) = self.records.get(&key) {
            if !existing.context.same_fence(&context)
                || existing.launch_profile.profile_id != profile_id
                || existing.launch_profile.validate().is_err()
                || existing.descriptor.validate().is_err()
                || existing.launch_profile.user_data != existing.descriptor.identity
                || existing.descriptor.provenance.owner != "gateway"
                || existing.descriptor.provenance.instance_id != context.instance_id
                || existing.descriptor.provenance.operation_id != operation_id
                || existing.descriptor.provenance.contract != LAUNCH_PROFILE_CONTRACT
            {
                return Err(UserDataProvisioningError::OperationConflict);
            }
            return Ok(outcome(existing));
        }
        if self.records.len() >= self.capacity {
            return Err(UserDataProvisioningError::CapacityExceeded);
        }
        let identity = UserDataIdentity::try_new(self.next_identity)
            .map_err(|_| UserDataProvisioningError::CapacityExceeded)?;
        self.next_identity = self
            .next_identity
            .checked_add(1)
            .ok_or(UserDataProvisioningError::CapacityExceeded)?;
        let descriptor = descriptor(&context, &operation_id, identity);
        let launch_profile = LaunchProfileBinding::try_new(profile_id, identity)
            .map_err(|_| UserDataProvisioningError::InvalidRequest)?;
        let pending = UserDataProvisioningRecord {
            operation_id: operation_id.clone(),
            context,
            descriptor: descriptor.clone(),
            launch_profile: launch_profile.clone(),
            status: UserDataProvisioningStatus::Pending,
            guidance: None,
        };
        self.store.insert(pending.clone())?;
        self.records.insert(key.clone(), pending);
        self.create_reserved(key, launch_profile, descriptor)
    }

    pub fn reconcile(
        &mut self,
        context: &SaveProfileContext,
        operation_id: &str,
    ) -> Result<UserDataProvisioningOutcome, UserDataProvisioningError> {
        context
            .validate()
            .map_err(|_| UserDataProvisioningError::InvalidRequest)?;
        validate_operation_id(operation_id)?;
        let key = (context.instance_id.clone(), operation_id.to_owned());
        let Some(record) = self.records.get(&key).cloned() else {
            return Err(UserDataProvisioningError::OperationNotFound);
        };
        if !record.context.same_fence(context) {
            return Err(UserDataProvisioningError::StaleContext);
        }
        if matches!(
            record.status,
            UserDataProvisioningStatus::Created | UserDataProvisioningStatus::Blocked
        ) {
            return Ok(outcome(&record));
        }
        let inspected = match self.port.inspect(record.descriptor.identity) {
            Ok(value) => value,
            Err(error) => return self.finish_port_error(key, error),
        };
        let (status, descriptor, guidance) = match inspected {
            UserDataInspection::Owned(provenance) if provenance == record.descriptor.provenance => {
                (
                    UserDataProvisioningStatus::Created,
                    Some(record.descriptor.clone()),
                    None,
                )
            }
            UserDataInspection::Absent => (
                UserDataProvisioningStatus::Unknown,
                None,
                Some(unknown_guidance()),
            ),
            UserDataInspection::UnknownContents => blocked_record("unknown existing contents"),
            UserDataInspection::Traversal => blocked_record("allocation path traversal rejected"),
            UserDataInspection::SymlinkEscape => {
                blocked_record("allocation symlink escape rejected")
            }
            UserDataInspection::Owned(_) => blocked_record("foreign allocation ownership"),
        };
        let updated = update_record(record, status, descriptor, guidance);
        self.store.update(updated.clone())?;
        self.records.insert(key, updated.clone());
        Ok(outcome(&updated))
    }

    pub fn operation(
        &self,
        instance_id: &str,
        operation_id: &str,
    ) -> Option<&UserDataProvisioningRecord> {
        self.records
            .get(&(instance_id.to_owned(), operation_id.to_owned()))
    }

    pub fn port(&self) -> &P {
        &self.port
    }

    pub fn port_mut(&mut self) -> &mut P {
        &mut self.port
    }

    pub fn into_store(self) -> S {
        self.store
    }

    fn create_reserved(
        &mut self,
        key: (String, String),
        launch_profile: LaunchProfileBinding,
        descriptor: UserDataDescriptor,
    ) -> Result<UserDataProvisioningOutcome, UserDataProvisioningError> {
        let inspected = match self.port.inspect(descriptor.identity) {
            Ok(value) => value,
            Err(error) => return self.finish_port_error(key, error),
        };
        if let Some((status, guidance)) = match inspected {
            UserDataInspection::Absent => None,
            UserDataInspection::UnknownContents => Some(blocked("unknown existing contents")),
            UserDataInspection::Traversal => Some(blocked("allocation path traversal rejected")),
            UserDataInspection::SymlinkEscape => {
                Some(blocked("allocation symlink escape rejected"))
            }
            UserDataInspection::Owned(_) => Some(blocked("implicit adoption is refused")),
        } {
            return self.finish(key, status, None, guidance);
        }
        let request = UserDataCreateRequest {
            descriptor: descriptor.clone(),
            launch_profile,
        };
        let created = match self.port.create(request) {
            Ok(value) => value,
            Err(UserDataPortError::TimeoutBeforeWrite | UserDataPortError::TimeoutAfterWrite) => {
                UserDataCreateOutcome::Unknown
            }
            Err(error) => return self.finish_port_error(key, error),
        };
        match created {
            UserDataCreateOutcome::Created(value)
            | UserDataCreateOutcome::AlreadyCreated(value) => {
                if value.identity != descriptor.identity
                    || value.provenance != descriptor.provenance
                    || value.validate().is_err()
                {
                    return self.finish(
                        key,
                        UserDataProvisioningStatus::Blocked,
                        None,
                        Some(guidance("allocation provenance mismatch")),
                    );
                }
                self.finish(key, UserDataProvisioningStatus::Created, Some(value), None)
            }
            UserDataCreateOutcome::TimeoutBeforeWrite
            | UserDataCreateOutcome::TimeoutAfterWrite
            | UserDataCreateOutcome::Unknown => self.finish(
                key,
                UserDataProvisioningStatus::Unknown,
                None,
                Some(unknown_guidance()),
            ),
        }
    }

    fn finish(
        &mut self,
        key: (String, String),
        status: UserDataProvisioningStatus,
        descriptor: Option<UserDataDescriptor>,
        guidance: Option<RecoveryGuidance>,
    ) -> Result<UserDataProvisioningOutcome, UserDataProvisioningError> {
        let Some(record) = self.records.get(&key).cloned() else {
            return Err(UserDataProvisioningError::OperationNotFound);
        };
        let updated = update_record(record, status, descriptor, guidance);
        self.store.update(updated.clone())?;
        self.records.insert(key, updated.clone());
        Ok(outcome(&updated))
    }

    fn finish_port_error(
        &mut self,
        key: (String, String),
        error: UserDataPortError,
    ) -> Result<UserDataProvisioningOutcome, UserDataProvisioningError> {
        let (status, guidance) = match error {
            UserDataPortError::UnknownContents => (
                UserDataProvisioningStatus::Blocked,
                Some(guidance("unknown existing contents")),
            ),
            UserDataPortError::Traversal => (
                UserDataProvisioningStatus::Blocked,
                Some(guidance("allocation path traversal rejected")),
            ),
            UserDataPortError::SymlinkEscape => (
                UserDataProvisioningStatus::Blocked,
                Some(guidance("allocation symlink escape rejected")),
            ),
            UserDataPortError::ExistingContents => (
                UserDataProvisioningStatus::Blocked,
                Some(guidance("implicit overwrite is refused")),
            ),
            UserDataPortError::TimeoutBeforeWrite | UserDataPortError::TimeoutAfterWrite => (
                UserDataProvisioningStatus::Unknown,
                Some(unknown_guidance()),
            ),
            UserDataPortError::Unavailable => (UserDataProvisioningStatus::Rejected, None),
        };
        self.finish(key, status, None, guidance)
    }
}
