// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::process_profile::{ApprovedLaunchProfiles, LaunchProfile};
use crate::{
    LaunchSpec, ProcessDescendantIdentity, ProcessFault, ProcessHandle, ProcessIdentity,
    ProcessLaunch, ProcessPort, ProcessState, StopMode,
};

/// Resolves opaque profile IDs before delegating to an injected process implementation.
///
/// The wrapped port never receives caller-provided commands, paths, URLs, or user-data settings.
/// It receives only the closed profile resolved by this server-owned catalog.
pub struct ApprovedLaunchProfileAdapter<P> {
    profiles: ApprovedLaunchProfiles,
    process: P,
    bindings: BTreeMap<ProcessHandle, LaunchProfile>,
}

impl<P> ApprovedLaunchProfileAdapter<P> {
    pub fn new(profiles: ApprovedLaunchProfiles, process: P) -> Self {
        Self {
            profiles,
            process,
            bindings: BTreeMap::new(),
        }
    }

    pub fn profiles(&self) -> &ApprovedLaunchProfiles {
        &self.profiles
    }

    pub fn process(&self) -> &P {
        &self.process
    }

    pub fn process_mut(&mut self) -> &mut P {
        &mut self.process
    }

    pub fn into_inner(self) -> P {
        self.process
    }

    fn profile_for(&self, specification: LaunchSpec) -> Result<LaunchProfile, ProcessFault> {
        let Some(id) = specification.profile_id() else {
            return Err(ProcessFault::ProfileRequired);
        };
        self.profiles
            .resolve(id)
            .map_err(|_| ProcessFault::ProfileNotApproved)
    }

    fn start_approved(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault>
    where
        P: ProcessPort,
    {
        let launch = self.process.start_with_profile(specification, profile)?;
        if launch
            .identity()
            .matches_profile(specification.instance_id(), profile)
        {
            self.bindings.insert(launch.identity().process(), profile);
        }
        Ok(launch)
    }

    fn validate_identity(
        &self,
        identity: &ProcessIdentity,
    ) -> Result<ProcessIdentity, ProcessFault> {
        let Some(profile) = self.bindings.get(&identity.process()) else {
            return Ok(identity.clone());
        };
        if identity.matches_profile(identity.instance_id(), *profile) {
            Ok(identity.clone())
        } else {
            Err(ProcessFault::IdentityMismatch)
        }
    }
}

impl<P: ProcessPort> ProcessPort for ApprovedLaunchProfileAdapter<P> {
    fn start(&mut self, specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        let profile = self.profile_for(specification)?;
        self.start_approved(specification, profile)
            .map(|launch| launch.identity().process())
    }

    fn start_with_profile(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        let approved = self.profile_for(specification)?;
        if approved != profile {
            return Err(ProcessFault::ProfileNotApproved);
        }
        self.start_approved(specification, profile)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.process.inspect(process)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        let identity = self.process.inspect_identity(process)?;
        self.validate_identity(&identity)
    }

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        self.process.stop(process, mode)?;
        self.bindings.remove(&process);
        Ok(())
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        self.process.descendants(process)
    }

    fn recover_owned(
        &mut self,
        instance_id: crate::InstanceId,
        profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        if self.profiles.resolve(profile.id()).ok() != Some(profile) {
            return Err(ProcessFault::ProfileNotApproved);
        }
        let Some(identity) = self.process.recover_owned(instance_id, profile)? else {
            return Ok(None);
        };
        if identity.matches_profile(instance_id, profile) {
            self.bindings.insert(identity.process(), profile);
        }
        Ok(Some(identity))
    }
}
