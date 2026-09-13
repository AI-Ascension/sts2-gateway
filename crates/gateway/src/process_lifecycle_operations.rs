// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleAction, LifecycleOperation, LifecycleOperationState};
use crate::{
    AuthorityEpoch, InstanceId, LifecycleError, LifecycleRequest, LifecycleResponse, ProcessPort,
};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn apply_request(
        &mut self,
        request: LifecycleRequest,
    ) -> Result<LifecycleResponse, LifecycleError> {
        self.authenticate(
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
        )?;
        let existing = self.record_for(request.instance_id(), request.operation_id());
        if let Some(operation) = existing {
            if !same_request(&operation, &request) {
                return Err(LifecycleError::OperationConflict);
            }
            return self.replay(operation);
        }
        let action = request.action().clone();
        self.validate_action(request.instance_id(), &action)?;
        self.reserve_action(request.instance_id(), &action)?;
        let sequence = self.issue_sequence()?;
        let mut operation = LifecycleOperation::new(
            request.operation_id(),
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
            action,
        );
        operation.set_sequence(sequence);
        self.persist_insert(operation.clone())?;
        if let LifecycleAction::LaunchNew { profile_id } = operation.action() {
            self.reserve_ownership(&operation, *profile_id)?;
        }
        self.execute(operation)
    }

    pub(crate) fn reconcile_operation(
        &mut self,
        lease: crate::LeaseProof,
        authority_epoch: AuthorityEpoch,
        operation_id: crate::OperationId,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let instance_id = lease.instance_id();
        self.authenticate(instance_id, lease, authority_epoch)?;
        let Some(operation) = self.record_for(instance_id, operation_id) else {
            return Err(LifecycleError::OperationNotFound);
        };
        if operation.lease() != lease {
            return Err(LifecycleError::Fence(crate::FenceFailure::WrongLease));
        }
        if operation.authority_epoch() != authority_epoch {
            return Err(LifecycleError::StaleAuthorityEpoch);
        }
        self.reconcile_record(operation)
    }

    fn validate_action(
        &self,
        instance_id: InstanceId,
        action: &LifecycleAction,
    ) -> Result<(), LifecycleError> {
        match action {
            LifecycleAction::LaunchNew { profile_id } | LifecycleAction::Restart { profile_id } => {
                self.profiles.resolve(*profile_id)?;
            }
            LifecycleAction::AttachExisting { identity } => {
                if identity.instance_id() != instance_id {
                    return Err(LifecycleError::IdentityMismatch);
                }
            }
            LifecycleAction::Stop { .. } => {}
        }
        Ok(())
    }

    fn reserve_action(
        &self,
        instance_id: InstanceId,
        action: &LifecycleAction,
    ) -> Result<(), LifecycleError> {
        match action {
            LifecycleAction::LaunchNew { .. } => {
                if self.ownership.contains_key(&instance_id)
                    || self.active_operation(instance_id).is_some()
                {
                    return Err(LifecycleError::InstanceBusy);
                }
                if self.occupied_count() >= self.config().max_processes() {
                    return Err(LifecycleError::CapacityExceeded);
                }
            }
            LifecycleAction::AttachExisting { identity } => {
                let Some(previous) = self.latest_authorized(instance_id) else {
                    return Err(LifecycleError::UnownedAttach);
                };
                if previous.process() != Some(identity) {
                    return Err(LifecycleError::IdentityMismatch);
                }
                if !matches!(
                    previous.state(),
                    LifecycleOperationState::Started | LifecycleOperationState::Attached
                ) {
                    return Err(LifecycleError::InstanceBusy);
                }
            }
            LifecycleAction::Stop { .. } | LifecycleAction::Restart { .. } => {
                let Some(previous) = self.latest_authorized(instance_id) else {
                    return Err(LifecycleError::InstanceNotFound);
                };
                let allowed = match action {
                    LifecycleAction::Stop { .. } => matches!(
                        previous.state(),
                        LifecycleOperationState::Started
                            | LifecycleOperationState::Attached
                            | LifecycleOperationState::Blocked
                    ),
                    LifecycleAction::Restart { .. } => matches!(
                        previous.state(),
                        LifecycleOperationState::Started
                            | LifecycleOperationState::Attached
                            | LifecycleOperationState::Blocked
                    ),
                    _ => false,
                };
                if !allowed {
                    return Err(LifecycleError::InstanceBusy);
                }
            }
        }
        Ok(())
    }

    fn execute(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        match operation.action().clone() {
            LifecycleAction::LaunchNew { profile_id } => self.execute_launch(operation, profile_id),
            LifecycleAction::AttachExisting { identity } => {
                self.execute_attach(operation, identity)
            }
            LifecycleAction::Stop { mode } => self.execute_stop(operation, mode),
            LifecycleAction::Restart { profile_id } => self.execute_restart(operation, profile_id),
        }
    }

    fn replay(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        match operation.state() {
            LifecycleOperationState::Started | LifecycleOperationState::Attached => {
                self.verify_record(operation)
            }
            LifecycleOperationState::IntentRecorded
            | LifecycleOperationState::Starting
            | LifecycleOperationState::Stopping
            | LifecycleOperationState::Restarting
            | LifecycleOperationState::Unknown
            | LifecycleOperationState::Blocked => self.reconcile_record(operation),
            LifecycleOperationState::Stopped
            | LifecycleOperationState::Failed
            | LifecycleOperationState::Rejected => Ok(LifecycleResponse::new(
                &operation,
                operation.state().lifecycle_state(),
            )),
        }
    }
}

fn same_request(operation: &LifecycleOperation, request: &LifecycleRequest) -> bool {
    operation.instance_id() == request.instance_id()
        && operation.lease() == request.lease()
        && operation.request_epoch() == request.authority_epoch()
        && operation.action() == request.action()
}
