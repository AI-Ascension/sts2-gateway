// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::process_ownership::LifecycleOwnership;
use crate::{
    AuthorityEpoch, InstanceId, LeaseProof, OperationId, ProcessFault, ProcessIdentity, StopMode,
};
use crate::{LaunchProfileId, LifecycleState};

#[path = "process_store_sqlite.rs"]
mod sqlite;
pub use sqlite::SqliteLifecycleStore;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub enum LifecycleAction {
    LaunchNew { profile_id: LaunchProfileId },
    AttachExisting { identity: ProcessIdentity },
    Stop { mode: StopMode },
    Restart { profile_id: LaunchProfileId },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LifecycleOperationState {
    IntentRecorded,
    Starting,
    Started,
    Attached,
    Stopping,
    Restarting,
    Stopped,
    Failed,
    Blocked,
    Rejected,
    Unknown,
}

impl LifecycleOperationState {
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::IntentRecorded
                | Self::Starting
                | Self::Started
                | Self::Attached
                | Self::Stopping
                | Self::Restarting
                | Self::Blocked
                | Self::Unknown
        )
    }

    pub const fn lifecycle_state(self) -> LifecycleState {
        match self {
            Self::IntentRecorded | Self::Starting | Self::Restarting => LifecycleState::Starting,
            // A running process is not gameplay-ready. The separate readiness
            // boundary owns the transition to `LifecycleState::Ready`.
            Self::Started | Self::Attached => LifecycleState::Starting,
            Self::Stopping => LifecycleState::Stopping,
            Self::Stopped => LifecycleState::Stopped,
            Self::Failed | Self::Blocked | Self::Rejected => LifecycleState::Failed,
            Self::Unknown => LifecycleState::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LifecycleFailure {
    CapacityExceeded,
    InstanceBusy,
    UnownedAttach,
    IdentityMismatch,
    ForeignDescendant,
    Process(ProcessFault),
    StaleAuthorityEpoch,
    Store,
    InvalidState(LifecycleState),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleOperation {
    operation_id: OperationId,
    instance_id: InstanceId,
    lease: LeaseProof,
    #[serde(default)]
    sequence: u64,
    request_epoch: AuthorityEpoch,
    authority_epoch: AuthorityEpoch,
    action: LifecycleAction,
    state: LifecycleOperationState,
    process: Option<ProcessIdentity>,
    failure: Option<LifecycleFailure>,
}

impl LifecycleOperation {
    pub fn new(
        operation_id: OperationId,
        instance_id: InstanceId,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        action: LifecycleAction,
    ) -> Self {
        Self {
            operation_id,
            instance_id,
            lease,
            sequence: 0,
            request_epoch: authority_epoch,
            authority_epoch,
            action,
            state: LifecycleOperationState::IntentRecorded,
            process: None,
            failure: None,
        }
    }

    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn lease(&self) -> LeaseProof {
        self.lease
    }

    /// Returns the gateway-issued ordering sequence. A value of zero denotes a
    /// legacy record written before sequence assignment was introduced.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority_epoch
    }

    pub const fn request_epoch(&self) -> AuthorityEpoch {
        self.request_epoch
    }

    pub const fn action(&self) -> &LifecycleAction {
        &self.action
    }

    pub const fn state(&self) -> LifecycleOperationState {
        self.state
    }

    pub const fn process(&self) -> Option<&ProcessIdentity> {
        self.process.as_ref()
    }

    pub const fn failure(&self) -> Option<LifecycleFailure> {
        self.failure
    }

    pub(crate) fn set_state(
        &mut self,
        state: LifecycleOperationState,
        process: Option<ProcessIdentity>,
        failure: Option<LifecycleFailure>,
    ) {
        self.state = state;
        self.process = process;
        self.failure = failure;
    }

    pub(crate) fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }

    pub(crate) fn set_authority_epoch(&mut self, epoch: AuthorityEpoch) {
        self.authority_epoch = epoch;
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LifecycleRecordKey {
    instance_id: InstanceId,
    operation_id: OperationId,
}

impl LifecycleRecordKey {
    pub const fn new(instance_id: InstanceId, operation_id: OperationId) -> Self {
        Self {
            instance_id,
            operation_id,
        }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LifecycleStoreError {
    Database,
    Serialization,
    NotFound,
    Conflict,
    Unsupported,
}

pub trait LifecycleRecordStore {
    fn get(
        &self,
        key: LifecycleRecordKey,
    ) -> Result<Option<LifecycleOperation>, LifecycleStoreError>;
    fn insert(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError>;
    fn update(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError>;
    fn list(&self) -> Result<Vec<LifecycleOperation>, LifecycleStoreError>;

    /// Returns the number of retained operation records without decoding them.
    ///
    /// The default keeps older stores source-compatible; built-in stores
    /// override it with a bounded count query.
    fn count(&self) -> Result<usize, LifecycleStoreError> {
        self.list().map(|records| records.len())
    }

    /// Returns durable authoritative ownership rows.
    ///
    /// A store that cannot persist ownership must fail closed when the
    /// lifecycle coordinator attempts to mutate ownership.
    fn list_ownership(&self) -> Result<Vec<LifecycleOwnership>, LifecycleStoreError> {
        Ok(Vec::new())
    }

    fn set_ownership(&mut self, _ownership: LifecycleOwnership) -> Result<(), LifecycleStoreError> {
        Err(LifecycleStoreError::Unsupported)
    }

    fn clear_ownership(&mut self, _instance_id: InstanceId) -> Result<(), LifecycleStoreError> {
        Err(LifecycleStoreError::Unsupported)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryLifecycleStore {
    records: BTreeMap<LifecycleRecordKey, LifecycleOperation>,
    ownership: BTreeMap<InstanceId, LifecycleOwnership>,
    fail_insert: bool,
    fail_update: bool,
    fail_update_after: Option<usize>,
}

impl InMemoryLifecycleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fail_insert(&mut self, value: bool) {
        self.fail_insert = value;
    }

    pub fn set_fail_update(&mut self, value: bool) {
        self.fail_update = value;
    }

    pub fn set_fail_update_after(&mut self, updates_before_failure: Option<usize>) {
        self.fail_update_after = updates_before_failure;
    }
}

impl LifecycleRecordStore for InMemoryLifecycleStore {
    fn get(
        &self,
        key: LifecycleRecordKey,
    ) -> Result<Option<LifecycleOperation>, LifecycleStoreError> {
        Ok(self.records.get(&key).cloned())
    }

    fn insert(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        if self.fail_insert {
            return Err(LifecycleStoreError::Database);
        }
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        if self.records.contains_key(&key) {
            return Err(LifecycleStoreError::Conflict);
        }
        self.records.insert(key, operation);
        Ok(())
    }

    fn update(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        if self.fail_update
            || self
                .fail_update_after
                .is_some_and(|remaining| remaining == 0)
        {
            return Err(LifecycleStoreError::Database);
        }
        if let Some(remaining) = self.fail_update_after.as_mut() {
            *remaining = remaining.saturating_sub(1);
        }
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        if !self.records.contains_key(&key) {
            return Err(LifecycleStoreError::NotFound);
        }
        self.records.insert(key, operation);
        Ok(())
    }

    fn list(&self) -> Result<Vec<LifecycleOperation>, LifecycleStoreError> {
        Ok(self.records.values().cloned().collect())
    }

    fn count(&self) -> Result<usize, LifecycleStoreError> {
        Ok(self.records.len())
    }

    fn list_ownership(&self) -> Result<Vec<LifecycleOwnership>, LifecycleStoreError> {
        Ok(self.ownership.values().cloned().collect())
    }

    fn set_ownership(&mut self, ownership: LifecycleOwnership) -> Result<(), LifecycleStoreError> {
        self.ownership.insert(ownership.instance_id(), ownership);
        Ok(())
    }

    fn clear_ownership(&mut self, instance_id: InstanceId) -> Result<(), LifecycleStoreError> {
        self.ownership.remove(&instance_id);
        Ok(())
    }
}
