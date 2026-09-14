// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::guidance::RecoveryGuidance;
use super::types::{
    LaunchProfileBinding, SaveProfileContext, UserDataDescriptor, UserDataIdentity,
    UserDataProvenance,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserDataInspection {
    Absent,
    Owned(UserDataProvenance),
    UnknownContents,
    Traversal,
    SymlinkEscape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataPortError {
    UnknownContents,
    Traversal,
    SymlinkEscape,
    ExistingContents,
    Unavailable,
    TimeoutBeforeWrite,
    TimeoutAfterWrite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserDataCreateRequest {
    pub descriptor: UserDataDescriptor,
    pub launch_profile: LaunchProfileBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserDataCreateOutcome {
    Created(UserDataDescriptor),
    AlreadyCreated(UserDataDescriptor),
    TimeoutBeforeWrite,
    TimeoutAfterWrite,
    Unknown,
}

/// The filesystem/process adapter for an isolated allocation.
///
/// Implementations own the physical root and must perform lexical and canonical containment
/// checks before touching it. The gateway passes only an opaque identity and provenance.
pub trait UserDataPort {
    fn inspect(
        &mut self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError>;

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserDataProvisioningRecord {
    pub operation_id: String,
    pub context: SaveProfileContext,
    pub descriptor: UserDataDescriptor,
    pub launch_profile: LaunchProfileBinding,
    pub status: UserDataProvisioningStatus,
    pub guidance: Option<RecoveryGuidance>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataProvisioningStatus {
    Pending,
    Created,
    Unknown,
    Blocked,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataProvisioningError {
    InvalidRequest,
    OperationConflict,
    CapacityExceeded,
    OperationNotFound,
    StaleContext,
    PersistenceFailed,
    Port(UserDataPortError),
}

pub trait UserDataRecordStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError>;
    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError>;
    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError>;
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryUserDataRecordStore {
    records: BTreeMap<(String, String), UserDataProvisioningRecord>,
}

impl UserDataRecordStore for InMemoryUserDataRecordStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        Ok(self.records.values().cloned().collect())
    }

    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let key = (
            record.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if self.records.contains_key(&key) {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        self.records.insert(key, record);
        Ok(())
    }

    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let key = (
            record.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if !self.records.contains_key(&key) {
            return Err(UserDataProvisioningError::OperationNotFound);
        }
        self.records.insert(key, record);
        Ok(())
    }
}

/// Deterministic in-memory adapter used by source/component tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryUserDataPort {
    entries: BTreeMap<UserDataIdentity, UserDataInspection>,
    outcomes: BTreeMap<UserDataIdentity, UserDataCreateOutcome>,
    inspect_errors: BTreeMap<UserDataIdentity, UserDataPortError>,
    create_errors: BTreeMap<UserDataIdentity, UserDataPortError>,
    pub inspected: Vec<UserDataIdentity>,
    pub created: Vec<UserDataIdentity>,
}

impl InMemoryUserDataPort {
    pub fn set_inspection(&mut self, identity: UserDataIdentity, value: UserDataInspection) {
        self.entries.insert(identity, value);
    }

    pub fn set_outcome(&mut self, identity: UserDataIdentity, value: UserDataCreateOutcome) {
        self.outcomes.insert(identity, value);
    }

    /// Fail `inspect` for this identity, as a real adapter would when it rejects a containment
    /// check instead of returning an inspection classification.
    pub fn set_inspect_error(&mut self, identity: UserDataIdentity, value: UserDataPortError) {
        self.inspect_errors.insert(identity, value);
    }

    /// Fail `create` for this identity, as a real adapter would when the root already holds
    /// unowned contents or is temporarily unavailable.
    pub fn set_create_error(&mut self, identity: UserDataIdentity, value: UserDataPortError) {
        self.create_errors.insert(identity, value);
    }

    pub fn inspection(&self, identity: UserDataIdentity) -> UserDataInspection {
        self.entries
            .get(&identity)
            .cloned()
            .unwrap_or(UserDataInspection::Absent)
    }
}

impl UserDataPort for InMemoryUserDataPort {
    fn inspect(
        &mut self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError> {
        self.inspected.push(identity);
        if let Some(error) = self.inspect_errors.get(&identity) {
            return Err(*error);
        }
        Ok(self.inspection(identity))
    }

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError> {
        let identity = request.descriptor.identity;
        self.created.push(identity);
        if let Some(error) = self.create_errors.get(&identity) {
            return Err(*error);
        }
        let outcome = self
            .outcomes
            .get(&identity)
            .cloned()
            .unwrap_or_else(|| UserDataCreateOutcome::Created(request.descriptor.clone()));
        if matches!(
            outcome,
            UserDataCreateOutcome::Created(_) | UserDataCreateOutcome::AlreadyCreated(_)
        ) {
            self.entries.insert(
                identity,
                UserDataInspection::Owned(request.descriptor.provenance),
            );
        }
        Ok(outcome)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserDataProvisioningOutcome {
    pub operation_id: String,
    pub status: UserDataProvisioningStatus,
    pub descriptor: Option<UserDataDescriptor>,
    pub guidance: Option<RecoveryGuidance>,
}

impl<T: UserDataPort + ?Sized> UserDataPort for Box<T> {
    fn inspect(
        &mut self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError> {
        (**self).inspect(identity)
    }

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError> {
        (**self).create(request)
    }
}

impl<T: UserDataRecordStore + ?Sized> UserDataRecordStore for Box<T> {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        (**self).list()
    }

    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        (**self).insert(record)
    }

    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        (**self).update(record)
    }
}
