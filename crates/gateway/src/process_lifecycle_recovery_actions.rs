// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleFailure, LifecycleOperation, LifecycleOperationState};
use crate::{
    LaunchProfile, LifecycleError, ProcessIdentity, ProcessLaunch, ProcessState, StopMode,
};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn reconcile_stop_intent(
        &mut self,
        mut operation: LifecycleOperation,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
        let Some(previous) = self
            .latest_authorized_excluding(operation.instance_id(), Some(operation.operation_id()))
        else {
            return self.block_operation(operation, LifecycleFailure::InstanceBusy);
        };
        let Some(identity) = previous.process().cloned() else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        self.profile_for_operation(&previous)?;
        operation.set_state(
            LifecycleOperationState::Stopping,
            Some(identity),
            operation.failure(),
        );
        self.persist_update(operation.clone())?;
        self.reconcile_stopping(operation)
    }

    pub(crate) fn reconcile_restart_intent(
        &mut self,
        mut operation: LifecycleOperation,
        profile_id: crate::LaunchProfileId,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
        let Some(previous) = self
            .latest_authorized_excluding(operation.instance_id(), Some(operation.operation_id()))
        else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        let Some(identity) = previous.process().cloned() else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        let profile = self.profiles.resolve(profile_id)?;
        if self.profile_for_operation(&previous)? != profile
            || !identity.matches_profile(operation.instance_id(), profile)
        {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        }
        operation = self.begin_restart(operation, identity)?;
        self.reconcile_restart(operation)
    }

    pub(crate) fn begin_restart(
        &mut self,
        mut operation: LifecycleOperation,
        identity: ProcessIdentity,
    ) -> Result<LifecycleOperation, LifecycleError> {
        let current = self.current_authority_epoch(operation.instance_id());
        let rotate = operation.request_epoch() == operation.authority_epoch();
        if operation.authority_epoch() != current {
            return Err(LifecycleError::StaleAuthorityEpoch);
        }
        if rotate {
            let Some(next) = current.next() else {
                return Err(LifecycleError::AuthorityExhausted);
            };
            self.set_authority_epoch(operation.instance_id(), next);
            operation.set_authority_epoch(next);
        }
        operation.set_state(
            LifecycleOperationState::Restarting,
            Some(identity),
            operation.failure(),
        );
        if let Err(error) = self.persist_update(operation.clone()) {
            if rotate {
                self.set_authority_epoch(operation.instance_id(), current);
            }
            return Err(error);
        }
        let profile_id = match operation.action() {
            crate::LifecycleAction::Restart { profile_id } => *profile_id,
            _ => return Err(LifecycleError::IdentityMismatch),
        };
        self.reserve_ownership(&operation, profile_id)?;
        Ok(operation)
    }

    pub(crate) fn finish_restart(
        &mut self,
        mut operation: LifecycleOperation,
        profile: LaunchProfile,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
        let Some(identity) = operation.process().cloned() else {
            return Err(LifecycleError::IdentityMismatch);
        };
        if !identity.matches_profile(operation.instance_id(), profile) {
            return self.reconcile_retained_cleanup(operation, identity);
        }
        let actual = match self.process.inspect_identity(identity.process()) {
            Ok(actual) => actual,
            Err(fault) => return self.process_failure(operation, fault),
        };
        if actual != identity {
            return self.block_cleanup(operation, LifecycleFailure::IdentityMismatch);
        }
        let state = match self.process.inspect(identity.process()) {
            Ok(state) => state,
            Err(fault) => return self.process_failure(operation, fault),
        };
        if matches!(state, ProcessState::Running) {
            if let Err(error) = self.verify_process(&identity, profile) {
                return self.block_cleanup_lifecycle_error(operation, error);
            }
            if let Err(fault) = self.process.stop(identity.process(), StopMode::Force) {
                return self.block_operation_with_process(operation, fault);
            }
        }
        let descendants = match self.process.descendants(identity.process()) {
            Ok(descendants) => descendants,
            Err(fault) => return self.process_failure(operation, fault),
        };
        if !descendants.is_empty() {
            return self.block_cleanup(operation, LifecycleFailure::ForeignDescendant);
        }
        operation.set_state(LifecycleOperationState::Restarting, None, None);
        self.persist_update(operation.clone())?;
        // Keep a durable identity-less reservation for the replacement until
        // recovery or a successful start establishes its exact ownership.
        self.reserve_ownership(&operation, profile.id())?;
        let specification = crate::LaunchSpec::for_profile(operation.instance_id(), profile.id());
        let launch = match self.process.start_with_profile(specification, profile) {
            Ok(launch) => launch,
            Err(fault) => return self.process_failure(operation, fault),
        };
        self.finish_start(operation, launch, profile, LifecycleOperationState::Started)
    }

    pub(crate) fn reconcile_launch(
        &mut self,
        operation: LifecycleOperation,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
        let profile = self.profile_for_operation(&operation)?;
        if let Some(identity) = operation.process().cloned()
            && !identity.matches_profile(operation.instance_id(), profile)
        {
            return self.reconcile_retained_cleanup(operation, identity);
        }
        if operation.process().is_some() {
            return self.verify_record(operation);
        }
        let previous_process = operation.process().cloned();
        let recovered = self.process.recover_owned(operation.instance_id(), profile);
        let identity = match recovered {
            Ok(Some(identity)) => identity,
            Ok(None) => return self.unknown(operation, previous_process, None),
            Err(fault) => {
                return self.unknown(
                    operation,
                    previous_process,
                    Some(LifecycleFailure::Process(fault)),
                );
            }
        };
        let launch = ProcessLaunch::new(identity);
        self.finish_recovered(operation, launch, profile)
    }

    /// Reconciles an adapter-retained identity that did not pass the approved
    /// profile check. The adapter retains the exact identity so cleanup can be
    /// retried without guessing a PID. We only stop after the current observed
    /// identity is byte-for-byte equal to the retained identity; identity drift
    /// leaves the operation blocked and the reservation held.
    pub(crate) fn reconcile_retained_cleanup(
        &mut self,
        mut operation: LifecycleOperation,
        identity: ProcessIdentity,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
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
        if actual != identity {
            operation.set_state(
                LifecycleOperationState::Blocked,
                Some(identity),
                Some(LifecycleFailure::IdentityMismatch),
            );
            self.persist_update(operation.clone())?;
            self.set_owner_process_for_operation(&operation, operation.process().cloned())?;
            return Err(LifecycleError::IdentityMismatch);
        }

        // The parent is intentionally allowed to fail the approved-profile
        // check here: this path exists to clean up an exact identity retained
        // by the adapter after a partial launch. Descendants still have to be
        // inside the operation's approved scope before a stop can have any
        // effect. Checking this observation first prevents a running retained
        // parent from being force-stopped while an unrelated child survives.
        let profile = self.profile_for_operation(&operation)?;
        let descendants = match self.process.descendants(identity.process()) {
            Ok(descendants) => descendants,
            Err(fault) => {
                return self.block_cleanup(operation, LifecycleFailure::Process(fault));
            }
        };
        if descendants.len() > profile.policy().max_descendants()
            || descendants
                .iter()
                .any(|descendant| !descendant.matches_profile(operation.instance_id(), profile))
        {
            return self.block_cleanup(operation, LifecycleFailure::ForeignDescendant);
        }

        match self.process.inspect(identity.process()) {
            Ok(ProcessState::Exited { .. }) => {
                self.finish_failed_launch_cleanup(operation, identity)
            }
            Ok(ProcessState::Running) => {
                if let Err(fault) = self.process.stop(identity.process(), StopMode::Force) {
                    return self.block_cleanup(operation, LifecycleFailure::Process(fault));
                }
                self.finish_failed_launch_cleanup(operation, identity)
            }
            Err(fault) => self.unknown(
                operation,
                Some(identity),
                Some(LifecycleFailure::Process(fault)),
            ),
        }
    }

    fn finish_failed_launch_cleanup(
        &mut self,
        mut operation: LifecycleOperation,
        identity: ProcessIdentity,
    ) -> Result<crate::LifecycleResponse, LifecycleError> {
        let descendants = match self.process.descendants(identity.process()) {
            Ok(descendants) => descendants,
            Err(fault) => return self.block_cleanup(operation, LifecycleFailure::Process(fault)),
        };
        if !descendants.is_empty() {
            return self.block_cleanup(operation, LifecycleFailure::ForeignDescendant);
        }
        operation.set_state(
            LifecycleOperationState::Failed,
            None,
            Some(LifecycleFailure::IdentityMismatch),
        );
        self.persist_update(operation.clone())?;
        self.clear_owned_for(&operation)?;
        Ok(crate::LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Failed,
        ))
    }
}
