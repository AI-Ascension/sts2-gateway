// SPDX-License-Identifier: MIT

use crate::identity::{AuthorityEpoch, FenceFailure, InstanceId, LeaseProof, OperationId};
use crate::process_profile::{LaunchProfileError, LaunchProfileId};
use crate::process_store::{
    LifecycleAction, LifecycleFailure, LifecycleOperation, LifecycleOperationState,
    LifecycleStoreError,
};
use crate::{ProcessFault, ProcessIdentity};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessLifecycleConfig {
    max_processes: usize,
}

impl ProcessLifecycleConfig {
    pub const fn new(max_processes: usize) -> Self {
        Self { max_processes }
    }

    pub const fn try_new(max_processes: usize) -> Result<Self, LifecycleError> {
        if max_processes == 0 {
            return Err(LifecycleError::CapacityExceeded);
        }
        Ok(Self::new(max_processes))
    }

    pub const fn max_processes(self) -> usize {
        self.max_processes
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LifecycleError {
    LeaseNotBound,
    Fence(FenceFailure),
    StaleAuthorityEpoch,
    AuthorityExhausted,
    CapacityExceeded,
    InstanceBusy,
    InstanceNotFound,
    OperationConflict,
    OperationNotFound,
    UnownedAttach,
    IdentityMismatch,
    ForeignDescendant,
    Process(ProcessFault),
    Profile(LaunchProfileError),
    Store(LifecycleStoreError),
    InvalidState(crate::LifecycleState),
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::LeaseNotBound => "lifecycle lease is not bound",
            Self::Fence(_) => "lifecycle lease fence rejected the request",
            Self::StaleAuthorityEpoch => "lifecycle authority epoch is stale",
            Self::AuthorityExhausted => "lifecycle authority epoch is exhausted",
            Self::CapacityExceeded => "lifecycle process capacity is exhausted",
            Self::InstanceBusy => "lifecycle instance already has an active operation",
            Self::InstanceNotFound => "lifecycle instance was not found",
            Self::OperationConflict => "lifecycle operation conflicts with a retained record",
            Self::OperationNotFound => "lifecycle operation was not found",
            Self::UnownedAttach => "process is not previously authorized by the gateway",
            Self::IdentityMismatch => "process identity did not match the authorized record",
            Self::ForeignDescendant => "process tree contains an unowned descendant",
            Self::Process(_) => "process adapter rejected the lifecycle operation",
            Self::Profile(_) => "launch profile is not approved",
            Self::Store(_) => "lifecycle operation store failed",
            Self::InvalidState(_) => "lifecycle operation is invalid in its current state",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for LifecycleError {}

impl From<LifecycleStoreError> for LifecycleError {
    fn from(error: LifecycleStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ProcessFault> for LifecycleError {
    fn from(error: ProcessFault) -> Self {
        Self::Process(error)
    }
}

impl From<LaunchProfileError> for LifecycleError {
    fn from(error: LaunchProfileError) -> Self {
        Self::Profile(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    operation_id: OperationId,
    instance_id: InstanceId,
    lease: LeaseProof,
    authority_epoch: AuthorityEpoch,
    action: LifecycleAction,
}

impl LifecycleRequest {
    pub fn launch_new(
        operation_id: OperationId,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        profile_id: LaunchProfileId,
    ) -> Self {
        Self {
            operation_id,
            instance_id: lease.instance_id(),
            lease,
            authority_epoch,
            action: LifecycleAction::LaunchNew { profile_id },
        }
    }

    pub fn attach_existing(
        operation_id: OperationId,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        identity: ProcessIdentity,
    ) -> Self {
        Self {
            operation_id,
            instance_id: lease.instance_id(),
            lease,
            authority_epoch,
            action: LifecycleAction::AttachExisting { identity },
        }
    }

    pub fn stop(
        operation_id: OperationId,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        mode: crate::StopMode,
    ) -> Self {
        Self {
            operation_id,
            instance_id: lease.instance_id(),
            lease,
            authority_epoch,
            action: LifecycleAction::Stop { mode },
        }
    }

    pub fn restart(
        operation_id: OperationId,
        lease: LeaseProof,
        authority_epoch: AuthorityEpoch,
        profile_id: LaunchProfileId,
    ) -> Self {
        Self {
            operation_id,
            instance_id: lease.instance_id(),
            lease,
            authority_epoch,
            action: LifecycleAction::Restart { profile_id },
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

    pub const fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority_epoch
    }

    pub const fn action(&self) -> &LifecycleAction {
        &self.action
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleResponse {
    operation_id: OperationId,
    instance_id: InstanceId,
    state: crate::LifecycleState,
    operation_state: LifecycleOperationState,
    process: Option<ProcessIdentity>,
    authority_epoch: AuthorityEpoch,
    failure: Option<LifecycleFailure>,
}

impl LifecycleResponse {
    pub(crate) fn new(operation: &LifecycleOperation, state: crate::LifecycleState) -> Self {
        Self {
            operation_id: operation.operation_id(),
            instance_id: operation.instance_id(),
            state,
            operation_state: operation.state(),
            process: operation.process().cloned(),
            authority_epoch: operation.authority_epoch(),
            failure: operation.failure(),
        }
    }

    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn state(&self) -> crate::LifecycleState {
        self.state
    }

    pub const fn operation_state(&self) -> LifecycleOperationState {
        self.operation_state
    }

    pub const fn process(&self) -> Option<&ProcessIdentity> {
        self.process.as_ref()
    }

    pub const fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority_epoch
    }

    pub const fn failure(&self) -> Option<LifecycleFailure> {
        self.failure
    }
}
