// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleAction, LifecycleFailure, LifecycleOperation};
use crate::{
    LaunchProfile, LifecycleError, ProcessFault, ProcessIdentity, ProcessPort, ProcessState,
};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn verify_process(
        &mut self,
        expected: &ProcessIdentity,
        profile: LaunchProfile,
    ) -> Result<(), LifecycleError> {
        if !expected.matches_profile(expected.instance_id(), profile) {
            return Err(LifecycleError::IdentityMismatch);
        }
        let actual = self.process.inspect_identity(expected.process())?;
        if actual != *expected {
            return Err(LifecycleError::IdentityMismatch);
        }
        match self.process.inspect(expected.process()) {
            Ok(ProcessState::Exited { .. }) => {
                return Err(LifecycleError::Process(ProcessFault::InspectionFailed));
            }
            Ok(ProcessState::Running) => {}
            Err(fault) => return Err(LifecycleError::Process(fault)),
        }
        self.verify_descendants(expected, profile)
            .map_err(|failure| match failure {
                LifecycleFailure::Process(fault) => LifecycleError::Process(fault),
                LifecycleFailure::ForeignDescendant => LifecycleError::ForeignDescendant,
                _ => LifecycleError::IdentityMismatch,
            })
    }

    pub(crate) fn verify_started_identity(
        &mut self,
        expected: &ProcessIdentity,
        profile: LaunchProfile,
    ) -> Result<(), LifecycleFailure> {
        let actual = self
            .process
            .inspect_identity(expected.process())
            .map_err(LifecycleFailure::Process)?;
        if actual != *expected {
            return Err(LifecycleFailure::IdentityMismatch);
        }
        if !expected.matches_profile(expected.instance_id(), profile) {
            return Err(LifecycleFailure::IdentityMismatch);
        }
        let state = self
            .process
            .inspect(expected.process())
            .map_err(LifecycleFailure::Process)?;
        if !matches!(state, ProcessState::Running) {
            return Err(LifecycleFailure::Process(ProcessFault::InspectionFailed));
        }
        self.verify_descendants(expected, profile)
    }

    fn verify_descendants(
        &mut self,
        expected: &ProcessIdentity,
        profile: LaunchProfile,
    ) -> Result<(), LifecycleFailure> {
        let descendants = self
            .process
            .descendants(expected.process())
            .map_err(LifecycleFailure::Process)?;
        if descendants.len() > profile.policy().max_descendants() {
            return Err(LifecycleFailure::ForeignDescendant);
        }
        if descendants
            .iter()
            .any(|descendant| !descendant.matches_profile(expected.instance_id(), profile))
        {
            return Err(LifecycleFailure::ForeignDescendant);
        }
        Ok(())
    }

    pub(crate) fn profile_for_operation(
        &self,
        operation: &LifecycleOperation,
    ) -> Result<LaunchProfile, LifecycleError> {
        match operation.action() {
            LifecycleAction::LaunchNew { profile_id } | LifecycleAction::Restart { profile_id } => {
                Ok(self.profiles.resolve(*profile_id)?)
            }
            LifecycleAction::AttachExisting { .. } | LifecycleAction::Stop { .. } => self
                .records
                .values()
                .filter(|candidate| {
                    candidate.instance_id() == operation.instance_id()
                        && candidate.process().is_some()
                        && candidate.state().is_active()
                })
                .rev()
                .find_map(|candidate| match candidate.action() {
                    LifecycleAction::LaunchNew { profile_id }
                    | LifecycleAction::Restart { profile_id } => {
                        self.profiles.resolve(*profile_id).ok()
                    }
                    LifecycleAction::AttachExisting { .. } | LifecycleAction::Stop { .. } => None,
                })
                .ok_or(LifecycleError::InstanceNotFound),
        }
    }
}
