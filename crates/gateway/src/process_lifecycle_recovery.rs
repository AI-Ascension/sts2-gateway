// SPDX-License-Identifier: MIT

use crate::process_store::{
    LifecycleAction, LifecycleFailure, LifecycleOperation, LifecycleOperationState,
};
use crate::{
    LaunchProfile, LifecycleError, LifecycleResponse, ProcessFault, ProcessIdentity, ProcessLaunch,
    ProcessState, StopMode,
};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn verify_record(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(identity) = operation.process().cloned() else {
            return Err(LifecycleError::IdentityMismatch);
        };
        let profile = self.profile_for_operation(&operation)?;
        if let Err(failure) = self.verify_started_identity(&identity, profile) {
            let inspection_failure = matches!(
                failure,
                LifecycleFailure::Process(ProcessFault::InspectionFailed)
            );
            let confirmed_cleanup =
                inspection_failure && self.confirm_exited_without_descendants(&identity);
            let state = if confirmed_cleanup {
                LifecycleOperationState::Failed
            } else if inspection_failure {
                // An inspection fault is not proof that the process disappeared.
                // Keep the identity and capacity reservation until a later
                // reconciliation can establish an exited, child-free process.
                LifecycleOperationState::Unknown
            } else {
                LifecycleOperationState::Blocked
            };
            let mut failed = operation;
            failed.set_state(state, Some(identity.clone()), Some(failure));
            self.persist_update(failed.clone())?;
            if confirmed_cleanup {
                self.clear_owned_for(&failed)?;
            } else if !matches!(failure, LifecycleFailure::IdentityMismatch) {
                self.set_owner_process_for_operation(&failed, Some(identity))?;
            }
            return Err(crate::process_lifecycle_failures::failure_error(failure));
        }
        let mut recovered = operation;
        if recovered.state() == LifecycleOperationState::Unknown {
            let state = match recovered.action() {
                LifecycleAction::AttachExisting { .. } => LifecycleOperationState::Attached,
                _ => LifecycleOperationState::Started,
            };
            recovered.set_state(state, Some(identity.clone()), None);
            self.persist_update(recovered.clone())?;
        }
        self.set_owned(&recovered, profile.id(), identity)?;
        Ok(LifecycleResponse::new(
            &recovered,
            recovered.state().lifecycle_state(),
        ))
    }

    pub(crate) fn reconcile_record(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        if operation.state().is_active() && !self.operation_is_current(&operation) {
            // Reconciliation is allowed to inspect only the current
            // authoritative operation. A newer terminal stop can clear the
            // ownership row while retaining this older record; do not let
            // that stale restart/unknown operation launch a replacement.
            return self.block_operation(operation, LifecycleFailure::InstanceBusy);
        }
        match operation.state() {
            LifecycleOperationState::IntentRecorded | LifecycleOperationState::Starting => {
                match operation.action().clone() {
                    LifecycleAction::LaunchNew { .. } => self.reconcile_launch(operation),
                    LifecycleAction::AttachExisting { identity } => {
                        self.execute_attach(operation, identity)
                    }
                    LifecycleAction::Stop { .. } => self.reconcile_stop_intent(operation),
                    LifecycleAction::Restart { profile_id } => {
                        self.reconcile_restart_intent(operation, profile_id)
                    }
                }
            }
            LifecycleOperationState::Unknown => match operation.action().clone() {
                LifecycleAction::LaunchNew { .. } => self.reconcile_launch(operation),
                LifecycleAction::AttachExisting { identity } => {
                    self.execute_attach(operation, identity)
                }
                LifecycleAction::Stop { .. } => {
                    if operation.process().is_some() {
                        self.reconcile_stopping(operation)
                    } else {
                        Ok(LifecycleResponse::new(
                            &operation,
                            crate::LifecycleState::Unknown,
                        ))
                    }
                }
                LifecycleAction::Restart { .. } => self.reconcile_restart(operation),
            },
            LifecycleOperationState::Started | LifecycleOperationState::Attached => {
                self.verify_record(operation)
            }
            LifecycleOperationState::Stopping => self.reconcile_stopping(operation),
            LifecycleOperationState::Restarting => self.reconcile_restart(operation),
            LifecycleOperationState::Blocked => {
                if matches!(operation.action(), LifecycleAction::Restart { .. }) {
                    self.reconcile_restart(operation)
                } else if matches!(operation.action(), LifecycleAction::LaunchNew { .. })
                    && operation.process().is_some()
                {
                    self.reconcile_launch(operation)
                } else if matches!(operation.action(), LifecycleAction::Stop { .. })
                    && operation.process().is_some()
                {
                    self.reconcile_stopping(operation)
                } else {
                    Ok(LifecycleResponse::new(
                        &operation,
                        operation.state().lifecycle_state(),
                    ))
                }
            }
            LifecycleOperationState::Stopped
            | LifecycleOperationState::Failed
            | LifecycleOperationState::Rejected => Ok(LifecycleResponse::new(
                &operation,
                operation.state().lifecycle_state(),
            )),
        }
    }

    pub(crate) fn reconcile_restart(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let profile = self.profile_for_operation(&operation)?;
        if operation.process().is_none() {
            let recovered = self.process.recover_owned(operation.instance_id(), profile);
            return match recovered {
                Ok(Some(identity)) => {
                    self.finish_recovered(operation, ProcessLaunch::new(identity), profile)
                }
                // The replacement may have been created after the last durable
                // transition but before the response was lost. Without an
                // explicit recovery observation, starting another process
                // would be a blind mutation and could duplicate the instance.
                Ok(None) => self.unknown(operation, None, None),
                Err(fault) => self.unknown(operation, None, Some(LifecycleFailure::Process(fault))),
            };
        }
        self.finish_restart(operation, profile)
    }

    pub(crate) fn finish_recovered(
        &mut self,
        mut operation: LifecycleOperation,
        launch: ProcessLaunch,
        profile: LaunchProfile,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let identity = launch.identity().clone();
        if !identity.matches_profile(operation.instance_id(), profile) {
            operation.set_state(
                LifecycleOperationState::Starting,
                Some(identity),
                Some(LifecycleFailure::IdentityMismatch),
            );
            return self.block_cleanup(operation, LifecycleFailure::IdentityMismatch);
        }
        if let Err(failure) = self.verify_started_identity(&identity, profile) {
            operation.set_state(
                LifecycleOperationState::Starting,
                Some(identity),
                Some(failure),
            );
            return self.block_cleanup(operation, failure);
        }
        operation.set_state(
            LifecycleOperationState::Started,
            Some(identity.clone()),
            None,
        );
        self.persist_update(operation.clone())?;
        self.set_owned(&operation, profile.id(), identity)?;
        Ok(LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Starting,
        ))
    }

    pub(crate) fn reconcile_stopping(
        &mut self,
        mut operation: LifecycleOperation,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(identity) = operation.process().cloned() else {
            return self.unknown(operation, None, None);
        };
        let profile = self.profile_for_operation(&operation)?;
        let actual = match self.process.inspect_identity(identity.process()) {
            Ok(actual) => actual,
            Err(fault) => {
                return self.unknown(
                    operation,
                    Some(identity),
                    Some(LifecycleFailure::Process(fault)),
                );
            }
        };
        if actual != identity || !identity.matches_profile(identity.instance_id(), profile) {
            operation.set_state(
                LifecycleOperationState::Blocked,
                Some(identity),
                Some(LifecycleFailure::IdentityMismatch),
            );
            self.persist_update(operation.clone())?;
            return Ok(LifecycleResponse::new(
                &operation,
                crate::LifecycleState::Unknown,
            ));
        }
        match self.process.inspect(identity.process()) {
            Ok(ProcessState::Exited { .. }) => {
                let descendants = match self.process.descendants(identity.process()) {
                    Ok(descendants) => descendants,
                    Err(fault) => {
                        return self.unknown(
                            operation,
                            Some(identity),
                            Some(LifecycleFailure::Process(fault)),
                        );
                    }
                };
                if !descendants.is_empty() {
                    operation.set_state(
                        LifecycleOperationState::Blocked,
                        Some(identity),
                        Some(LifecycleFailure::ForeignDescendant),
                    );
                    self.persist_update(operation.clone())?;
                    return Ok(LifecycleResponse::new(
                        &operation,
                        crate::LifecycleState::Failed,
                    ));
                }
                operation.set_state(LifecycleOperationState::Stopped, None, None);
                self.persist_update(operation.clone())?;
                self.clear_owned_for(&operation)?;
                Ok(LifecycleResponse::new(
                    &operation,
                    crate::LifecycleState::Stopped,
                ))
            }
            Ok(ProcessState::Running) => {
                if let Err(error) = self.verify_process(&identity, profile) {
                    return self.block_cleanup_lifecycle_error(operation, error);
                }
                let mode = match operation.action() {
                    LifecycleAction::Stop { mode } => *mode,
                    _ => StopMode::Force,
                };
                self.finish_stop(operation, identity, mode)
            }
            Err(fault) => self.unknown(
                operation,
                Some(identity),
                Some(LifecycleFailure::Process(fault)),
            ),
        }
    }

    pub(crate) fn unknown(
        &mut self,
        mut operation: LifecycleOperation,
        process: Option<ProcessIdentity>,
        failure: Option<LifecycleFailure>,
    ) -> Result<LifecycleResponse, LifecycleError> {
        operation.set_state(LifecycleOperationState::Unknown, process, failure);
        self.persist_update(operation.clone())?;
        Ok(LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Unknown,
        ))
    }
}
