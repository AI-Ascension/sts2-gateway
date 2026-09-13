// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleFailure, LifecycleOperation, LifecycleOperationState};
use crate::{
    LaunchProfile, LaunchProfileId, LaunchSpec, LifecycleError, LifecycleResponse, ProcessIdentity,
    ProcessLaunch, ProcessPort, StopMode,
};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn execute_launch(
        &mut self,
        mut operation: LifecycleOperation,
        profile_id: LaunchProfileId,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let profile = self.profiles.resolve(profile_id)?;
        operation.set_state(LifecycleOperationState::Starting, None, None);
        self.persist_update(operation.clone())?;
        let specification = LaunchSpec::for_profile(operation.instance_id(), profile_id);
        let launch = match self.process.start_with_profile(specification, profile) {
            Ok(launch) => launch,
            Err(fault) => return self.process_failure(operation, fault),
        };
        self.finish_start(operation, launch, profile, LifecycleOperationState::Started)
    }

    pub(crate) fn finish_start(
        &mut self,
        mut operation: LifecycleOperation,
        launch: ProcessLaunch,
        profile: LaunchProfile,
        state: LifecycleOperationState,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let identity = launch.identity().clone();
        if !identity.matches_profile(operation.instance_id(), profile) {
            operation.set_state(
                LifecycleOperationState::Starting,
                Some(identity),
                Some(LifecycleFailure::IdentityMismatch),
            );
            return self.finish_start_failure(operation, LifecycleFailure::IdentityMismatch);
        }
        if let Err(failure) = self.verify_started_identity(&identity, profile) {
            operation.set_state(
                LifecycleOperationState::Starting,
                Some(identity),
                Some(failure),
            );
            return self.finish_start_failure(operation, failure);
        }
        operation.set_state(state, Some(identity.clone()), None);
        self.persist_update(operation.clone())?;
        self.set_owned(&operation, profile.id(), identity)?;
        Ok(LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Starting,
        ))
    }

    fn finish_start_failure(
        &mut self,
        mut operation: LifecycleOperation,
        failure: LifecycleFailure,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(identity) = operation.process().cloned() else {
            return self.block_operation(operation, failure);
        };
        operation.set_state(
            LifecycleOperationState::Starting,
            Some(identity.clone()),
            Some(failure),
        );
        match self.process.stop(identity.process(), StopMode::Force) {
            Err(fault) => self.block_cleanup(operation, LifecycleFailure::Process(fault)),
            Ok(()) => {
                let descendants = match self.process.descendants(identity.process()) {
                    Ok(descendants) => descendants,
                    Err(fault) => {
                        return self.block_cleanup(operation, LifecycleFailure::Process(fault));
                    }
                };
                if !descendants.is_empty() {
                    return self.block_cleanup(operation, LifecycleFailure::ForeignDescendant);
                }
                self.block_operation(operation, failure)
            }
        }
    }

    pub(crate) fn execute_attach(
        &mut self,
        mut operation: LifecycleOperation,
        identity: ProcessIdentity,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(previous) = self
            .latest_authorized_excluding(operation.instance_id(), Some(operation.operation_id()))
        else {
            return self.block_operation(operation, LifecycleFailure::UnownedAttach);
        };
        let Some(expected) = previous.process().cloned() else {
            return self.block_operation(operation, LifecycleFailure::UnownedAttach);
        };
        if expected != identity {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        }
        let profile = self.profile_for_operation(&previous)?;
        operation.set_state(
            LifecycleOperationState::Attached,
            Some(identity.clone()),
            None,
        );
        self.persist_update(operation.clone())?;
        if let Err(error) = self.verify_process(&identity, profile) {
            return self.block_cleanup_lifecycle_error(operation, error);
        }
        self.set_owned(&operation, profile.id(), identity)?;
        self.persist_update(operation.clone())?;
        Ok(LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Starting,
        ))
    }

    pub(crate) fn execute_stop(
        &mut self,
        mut operation: LifecycleOperation,
        mode: StopMode,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(previous) = self
            .latest_authorized_excluding(operation.instance_id(), Some(operation.operation_id()))
        else {
            return self.block_operation(operation, LifecycleFailure::InstanceBusy);
        };
        let Some(identity) = previous.process().cloned() else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        let profile = self.profile_for_operation(&previous)?;
        operation.set_state(
            LifecycleOperationState::Stopping,
            Some(identity.clone()),
            None,
        );
        self.persist_update(operation.clone())?;
        if let Err(error) = self.verify_process(&identity, profile) {
            return self.block_cleanup_lifecycle_error(operation, error);
        }
        self.finish_stop(operation, identity, mode)
    }

    pub(crate) fn finish_stop(
        &mut self,
        mut operation: LifecycleOperation,
        identity: ProcessIdentity,
        mode: StopMode,
    ) -> Result<LifecycleResponse, LifecycleError> {
        if let Err(fault) = self.process.stop(identity.process(), mode) {
            return self.block_operation_with_process(operation, fault);
        }
        let descendants = match self.process.descendants(identity.process()) {
            Ok(descendants) => descendants,
            Err(fault) => return self.block_operation_with_process(operation, fault),
        };
        if !descendants.is_empty() {
            return self.block_cleanup(operation, LifecycleFailure::ForeignDescendant);
        }
        operation.set_state(LifecycleOperationState::Stopped, None, None);
        self.persist_update(operation.clone())?;
        self.clear_owned(operation.instance_id())?;
        Ok(LifecycleResponse::new(
            &operation,
            crate::LifecycleState::Stopped,
        ))
    }

    pub(crate) fn execute_restart(
        &mut self,
        mut operation: LifecycleOperation,
        profile_id: LaunchProfileId,
    ) -> Result<LifecycleResponse, LifecycleError> {
        let Some(previous) = self
            .latest_authorized_excluding(operation.instance_id(), Some(operation.operation_id()))
        else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        let Some(identity) = previous.process().cloned() else {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        };
        let profile = self.profiles.resolve(profile_id)?;
        let previous_profile = self.profile_for_operation(&previous)?;
        operation.set_state(
            LifecycleOperationState::Restarting,
            Some(identity.clone()),
            None,
        );
        self.persist_update(operation.clone())?;
        if profile != previous_profile {
            return self.block_operation(operation, LifecycleFailure::IdentityMismatch);
        }
        if let Err(error) = self.verify_process(&identity, profile) {
            return self.block_cleanup_lifecycle_error(operation, error);
        }
        operation = self.begin_restart(operation, identity)?;
        self.finish_restart(operation, profile)
    }
}
