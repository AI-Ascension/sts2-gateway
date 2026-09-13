// SPDX-License-Identifier: MIT

use crate::process_store::{
    LifecycleAction, LifecycleFailure, LifecycleOperation, LifecycleOperationState,
};
use crate::{LifecycleError, LifecycleResponse, ProcessFault};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn process_failure(
        &mut self,
        operation: LifecycleOperation,
        fault: ProcessFault,
    ) -> Result<LifecycleResponse, LifecycleError> {
        if operation.process().is_none() {
            let error = LifecycleError::Process(fault);
            // `StartRejected` and `ProfileRequired` are explicit pre-start
            // outcomes. Every other launch fault may have happened after a
            // child was created (for example while inspecting or cleaning
            // it), so retaining a terminal `Failed` row would release the
            // instance for a duplicate launch.
            if !matches!(
                fault,
                ProcessFault::StartRejected
                    | ProcessFault::ProfileRequired
                    | ProcessFault::ProfileNotApproved
            ) {
                let profile = match operation.action() {
                    LifecycleAction::LaunchNew { profile_id }
                    | LifecycleAction::Restart { profile_id } => {
                        self.profiles.resolve(*profile_id).ok()
                    }
                    LifecycleAction::AttachExisting { .. } | LifecycleAction::Stop { .. } => None,
                };
                let recovered = profile.and_then(|profile| {
                    self.process
                        .recover_owned(operation.instance_id(), profile)
                        .ok()
                        .flatten()
                        .filter(|identity| {
                            identity.matches_profile(operation.instance_id(), profile)
                        })
                });
                let mut unknown = operation;
                unknown.set_state(
                    LifecycleOperationState::Unknown,
                    recovered.clone(),
                    Some(LifecycleFailure::Process(fault)),
                );
                self.persist_update(unknown.clone())?;
                self.set_owner_process_for_operation(&unknown, recovered)?;
                return Err(error);
            }
            let mut failed = operation;
            failed.set_state(
                LifecycleOperationState::Failed,
                None,
                Some(LifecycleFailure::Process(fault)),
            );
            self.persist_update(failed.clone())?;
            self.clear_owned_for(&failed)?;
            return Err(error);
        }
        self.block_operation_with_process(operation, fault)
    }

    pub(crate) fn block_cleanup_lifecycle_error(
        &mut self,
        operation: LifecycleOperation,
        error: LifecycleError,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let failure = match error {
            LifecycleError::ForeignDescendant => LifecycleFailure::ForeignDescendant,
            LifecycleError::IdentityMismatch => LifecycleFailure::IdentityMismatch,
            LifecycleError::Process(fault) => LifecycleFailure::Process(fault),
            other => return Err(other),
        };
        self.block_cleanup(operation, failure)
    }

    pub(crate) fn block_operation_with_process(
        &mut self,
        operation: LifecycleOperation,
        fault: ProcessFault,
    ) -> Result<LifecycleResponse, LifecycleError> {
        self.block_operation(operation, LifecycleFailure::Process(fault))
    }

    pub(crate) fn block_operation(
        &mut self,
        mut operation: LifecycleOperation,
        failure: LifecycleFailure,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let error = failure_error(failure);
        let state = if matches!(failure, LifecycleFailure::Process(_)) {
            LifecycleOperationState::Blocked
        } else {
            LifecycleOperationState::Rejected
        };
        let process = operation.process().cloned();
        let clear_owner = matches!(state, LifecycleOperationState::Rejected)
            && self
                .ownership
                .get(&operation.instance_id())
                .is_some_and(|owner| owner.operation_id() == operation.operation_id());
        operation.set_state(state, process, Some(failure));
        self.persist_update(operation.clone())?;
        if clear_owner {
            self.clear_owned_for(&operation)?;
        }
        Err(error)
    }

    pub(crate) fn block_cleanup(
        &mut self,
        mut operation: LifecycleOperation,
        failure: LifecycleFailure,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let error = failure_error(failure);
        let process = operation.process().cloned();
        operation.set_state(LifecycleOperationState::Blocked, process, Some(failure));
        // Preserve an identity-bearing cleanup obligation even if durable
        // persistence is temporarily unavailable. Capacity must remain held
        // until a later reconciliation can prove the tree is gone.
        self.set_owner_process_for_operation(&operation, operation.process().cloned())?;
        self.persist_update(operation)?;
        Err(error)
    }
}

pub(crate) fn failure_error(failure: LifecycleFailure) -> LifecycleError {
    match failure {
        LifecycleFailure::CapacityExceeded => LifecycleError::CapacityExceeded,
        LifecycleFailure::InstanceBusy => LifecycleError::InstanceBusy,
        LifecycleFailure::UnownedAttach => LifecycleError::UnownedAttach,
        LifecycleFailure::IdentityMismatch => LifecycleError::IdentityMismatch,
        LifecycleFailure::ForeignDescendant => LifecycleError::ForeignDescendant,
        LifecycleFailure::Process(fault) => LifecycleError::Process(fault),
        LifecycleFailure::StaleAuthorityEpoch => LifecycleError::StaleAuthorityEpoch,
        LifecycleFailure::Store => LifecycleError::Store(crate::LifecycleStoreError::Database),
        LifecycleFailure::InvalidState(state) => LifecycleError::InvalidState(state),
    }
}
