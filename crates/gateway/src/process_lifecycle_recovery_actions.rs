// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleFailure, LifecycleOperation, LifecycleOperationState};
use crate::{LaunchProfile, LifecycleError, ProcessIdentity, ProcessState, StopMode};

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
        let actual = match self.process.inspect_identity(identity.process()) {
            Ok(actual) => actual,
            Err(fault) => return self.process_failure(operation, fault),
        };
        if actual != identity || !identity.matches_profile(operation.instance_id(), profile) {
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
        self.clear_owned(operation.instance_id());
        operation.set_state(LifecycleOperationState::Restarting, None, None);
        self.persist_update(operation.clone())?;
        let specification = crate::LaunchSpec::for_profile(operation.instance_id(), profile.id());
        let launch = match self.process.start_with_profile(specification, profile) {
            Ok(launch) => launch,
            Err(fault) => return self.process_failure(operation, fault),
        };
        self.finish_start(operation, launch, profile, LifecycleOperationState::Started)
    }
}
