// SPDX-License-Identifier: MIT

use crate::{
    InstanceId, LaunchProfile, LaunchSpec, ProcessFault, ProcessHandle, ProcessIdentity,
    ProcessPort, ProcessState, StopMode,
};

use super::{ProcessSupervisor, ProcessSupervisorError};

impl<P: ProcessPort> ProcessSupervisor<P> {
    /// Retains a launched identity while cleanup is unresolved. A launch
    /// identity is the only handle the supervisor has after an identity or
    /// descendant check fails, so dropping it would make a later cleanup
    /// impossible and could permit a duplicate process.
    fn retain_identity(&mut self, instance_id: InstanceId, identity: ProcessIdentity) {
        self.identities.insert(instance_id, identity.clone());
        self.owned.insert(instance_id, identity.process());
    }

    /// Cleans up a launch whose returned identity was not authorized. Cleanup
    /// failures and surviving descendants retain the handle as owned; only an
    /// observed, child-free stop releases the reservation.
    fn reject_launched_identity(
        &mut self,
        instance_id: InstanceId,
        identity: ProcessIdentity,
    ) -> ProcessSupervisorError {
        let process = identity.process();
        match self.process.stop(process, StopMode::Force) {
            Err(fault) => {
                self.retain_identity(instance_id, identity);
                ProcessSupervisorError::Process(fault)
            }
            Ok(()) => match self.process.descendants(process) {
                Err(fault) => {
                    self.retain_identity(instance_id, identity);
                    ProcessSupervisorError::Process(fault)
                }
                Ok(descendants) if !descendants.is_empty() => {
                    self.retain_identity(instance_id, identity);
                    ProcessSupervisorError::ForeignDescendant
                }
                Ok(_) => {
                    self.identities.remove(&instance_id);
                    self.owned.remove(&instance_id);
                    ProcessSupervisorError::IdentityMismatch
                }
            },
        }
    }

    pub fn start_resolved(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessIdentity, ProcessSupervisorError> {
        let instance_id = specification.instance_id();
        if specification.profile_id() != Some(profile.id()) {
            return Err(ProcessSupervisorError::Process(
                ProcessFault::ProfileNotApproved,
            ));
        }
        if self.owned.contains_key(&instance_id) {
            return Err(ProcessSupervisorError::AlreadyOwned);
        }
        if self.owned.len() >= self.config.max_owned_processes() {
            return Err(ProcessSupervisorError::CapacityExceeded);
        }
        let launch = self
            .process
            .start_with_profile(specification, profile)
            .map_err(ProcessSupervisorError::Process)?;
        if !launch.identity().matches_profile(instance_id, profile) {
            let identity = launch.identity().clone();
            return Err(self.reject_launched_identity(instance_id, identity));
        }
        self.retain_identity(instance_id, launch.identity().clone());
        Ok(launch.identity().clone())
    }

    /// Attaches only after a caller has supplied an identity from a prior gateway record.
    pub fn attach_authorized(
        &mut self,
        identity: ProcessIdentity,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        let instance_id = identity.instance_id();
        if self.owned.contains_key(&instance_id) {
            return Err(ProcessSupervisorError::AlreadyOwned);
        }
        if self.owned.len() >= self.config.max_owned_processes() {
            return Err(ProcessSupervisorError::CapacityExceeded);
        }
        if self.identities.get(&instance_id) != Some(&identity) {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        let actual = self
            .process
            .inspect_identity(identity.process())
            .map_err(ProcessSupervisorError::Process)?;
        if actual != identity {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        if !matches!(
            self.process
                .inspect(identity.process())
                .map_err(ProcessSupervisorError::Process)?,
            ProcessState::Running
        ) {
            return Err(ProcessSupervisorError::Process(
                ProcessFault::InspectionFailed,
            ));
        }
        self.identities.insert(instance_id, identity.clone());
        self.owned.insert(instance_id, identity.process());
        Ok(identity.process())
    }

    pub fn stop_verified(
        &mut self,
        instance_id: InstanceId,
        expected: &ProcessIdentity,
        mode: StopMode,
    ) -> Result<(), ProcessSupervisorError> {
        let handle = self
            .owned
            .get(&instance_id)
            .copied()
            .ok_or(ProcessSupervisorError::NotOwned)?;
        if handle != expected.process() || expected.instance_id() != instance_id {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        let actual = self
            .process
            .inspect_identity(handle)
            .map_err(ProcessSupervisorError::Process)?;
        if actual != *expected {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        if !self
            .process
            .descendants(handle)
            .map_err(ProcessSupervisorError::Process)?
            .is_empty()
        {
            return Err(ProcessSupervisorError::ForeignDescendant);
        }
        self.stop(instance_id, mode)
    }

    /// Restarts an owned process with a server-resolved profile and a fresh identity.
    pub fn restart_resolved(
        &mut self,
        instance_id: InstanceId,
        profile: LaunchProfile,
    ) -> Result<ProcessIdentity, ProcessSupervisorError> {
        let old_handle = self
            .owned
            .get(&instance_id)
            .copied()
            .ok_or(ProcessSupervisorError::NotOwned)?;
        let expected = self
            .identities
            .get(&instance_id)
            .cloned()
            .ok_or(ProcessSupervisorError::IdentityMismatch)?;
        if expected.instance_id() != instance_id || !expected.matches_profile(instance_id, profile)
        {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        let actual = self
            .process
            .inspect_identity(old_handle)
            .map_err(ProcessSupervisorError::Process)?;
        if actual != expected {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        if !self
            .process
            .descendants(old_handle)
            .map_err(ProcessSupervisorError::Process)?
            .is_empty()
        {
            return Err(ProcessSupervisorError::ForeignDescendant);
        }
        self.process
            .stop(old_handle, StopMode::Force)
            .map_err(ProcessSupervisorError::Process)?;
        if !self
            .process
            .descendants(old_handle)
            .map_err(ProcessSupervisorError::Process)?
            .is_empty()
        {
            return Err(ProcessSupervisorError::ForeignDescendant);
        }
        self.identities.remove(&instance_id);
        self.owned.remove(&instance_id);
        let specification = LaunchSpec::for_profile(instance_id, profile.id());
        let launch = self
            .process
            .start_with_profile(specification, profile)
            .map_err(ProcessSupervisorError::Process)?;
        if !launch.identity().matches_profile(instance_id, profile) {
            let identity = launch.identity().clone();
            return Err(self.reject_launched_identity(instance_id, identity));
        }
        let identity = launch.identity().clone();
        self.retain_identity(instance_id, identity.clone());
        Ok(identity)
    }
}
