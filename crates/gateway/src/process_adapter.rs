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
    /// Launch identities that were created by this adapter but could not yet
    /// be cleaned up. The profile may be mismatched; the identity is still a
    /// durable cleanup witness and must not be dropped.
    unresolved: BTreeMap<ProcessHandle, ProcessIdentity>,
}

impl<P> ApprovedLaunchProfileAdapter<P> {
    pub fn new(profiles: ApprovedLaunchProfiles, process: P) -> Self {
        Self {
            profiles,
            process,
            bindings: BTreeMap::new(),
            unresolved: BTreeMap::new(),
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
        if !launch
            .identity()
            .matches_profile(specification.instance_id(), profile)
        {
            // Keep the handle bound while cleanup is unresolved. Returning
            // only `ProcessFault` cannot transfer the launch to the caller,
            // so the adapter must retain enough local ownership to retry a
            // failed stop instead of silently abandoning the process.
            let identity = launch.identity().clone();
            let process = identity.process();
            self.bindings.insert(process, profile);
            self.unresolved.insert(process, identity.clone());
            return match self.process.stop(process, StopMode::Force) {
                Err(fault) => Err(fault),
                Ok(()) => match self.process.descendants(process) {
                    Err(fault) => Err(fault),
                    Ok(descendants) if !descendants.is_empty() => {
                        Err(ProcessFault::DescendantOutOfScope)
                    }
                    Ok(_) => {
                        self.bindings.remove(&process);
                        self.unresolved.remove(&process);
                        Err(ProcessFault::IdentityMismatch)
                    }
                },
            };
        }
        let process = launch.identity().process();
        self.bindings.insert(process, profile);
        self.unresolved.remove(&process);
        Ok(launch)
    }

    fn validate_identity(
        &self,
        identity: &ProcessIdentity,
    ) -> Result<ProcessIdentity, ProcessFault> {
        // A retained launch identity is an adapter-created cleanup obligation.
        // It must be observable even when its executable or namespace failed
        // profile validation; the lifecycle coordinator will fence cleanup by
        // comparing the exact identity before stopping it.
        if self.unresolved.contains_key(&identity.process()) {
            return Ok(identity.clone());
        }
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
        self.unresolved.remove(&process);
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
        if let Some(identity) = self
            .unresolved
            .values()
            .find(|identity| identity.instance_id() == instance_id)
            .cloned()
        {
            return Ok(Some(identity));
        }
        let Some(identity) = self.process.recover_owned(instance_id, profile)? else {
            return Ok(None);
        };
        if identity.instance_id() != instance_id {
            return Err(ProcessFault::IdentityMismatch);
        }
        self.bindings.insert(identity.process(), profile);
        if !identity.matches_profile(instance_id, profile) {
            self.unresolved.insert(identity.process(), identity.clone());
        } else {
            self.unresolved.remove(&identity.process());
        }
        Ok(Some(identity))
    }
}
