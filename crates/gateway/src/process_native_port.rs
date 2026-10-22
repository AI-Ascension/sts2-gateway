// SPDX-License-Identifier: MIT

use super::configuration::NativeProfile;
use super::store::{GenerationRegistry, generation_key};
use super::*;
use std::collections::BTreeMap;

#[path = "process_native_observation.rs"]
mod observation;

pub(super) struct NativeProcessPort<C: ServiceController> {
    profiles: BTreeMap<u64, NativeProfile>,
    pub(super) controller: C,
    pub(super) registry: GenerationRegistry,
}

impl<C: ServiceController> NativeProcessPort<C> {
    pub(super) fn new(
        profiles: BTreeMap<u64, NativeProfile>,
        controller: C,
        registry: GenerationRegistry,
    ) -> Self {
        Self {
            profiles,
            controller,
            registry,
        }
    }

    fn profile(&self, profile: LaunchProfile) -> Result<NativeProfile, ProcessFault> {
        self.profiles
            .get(&profile.id().value())
            .filter(|binding| binding.profile == profile)
            .cloned()
            .ok_or(ProcessFault::ProfileNotApproved)
    }

    fn checked_record(&self, record: &GenerationRecord) -> Result<NativeProfile, ProcessFault> {
        let profile = self.profile(record.profile)?;
        if record.boot_id != self.controller.boot_id() || profile.digest != record.binding_digest {
            return Err(ProcessFault::IdentityMismatch);
        }
        Ok(profile)
    }

    fn current(
        &mut self,
        handle: ProcessHandle,
    ) -> Result<(GenerationRecord, Vec<NativeMember>), ProcessFault> {
        let mut record = self
            .registry
            .by_handle(handle.value())
            .map_err(|_| ProcessFault::IdentityMismatch)?;
        let instance = record
            .identity
            .as_ref()
            .ok_or(ProcessFault::InspectionFailed)?
            .instance_id();
        let members = self.observe(&mut record, instance)?;
        Ok((record, members))
    }

    fn recovered(
        &mut self,
        mut record: GenerationRecord,
        instance: InstanceId,
    ) -> Result<GenerationRecovery, ProcessFault> {
        self.checked_record(&record)?;
        if !record.dispatched {
            return Ok(GenerationRecovery::Indeterminate);
        }
        self.observe(&mut record, instance)?;
        match record.identity {
            Some(identity) => Ok(GenerationRecovery::Found(identity)),
            None => Ok(GenerationRecovery::Indeterminate),
        }
    }
}

impl<C: ServiceController> ProcessPort for NativeProcessPort<C> {
    fn start(&mut self, _: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn start_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, GenerationStartError> {
        if specification.profile_id() != Some(profile.id())
            || specification.instance_id() != generation.instance_id()
            || generation.profile_id() != profile.id()
        {
            return Err(GenerationStartError::Unsupported);
        }
        let binding = self
            .profile(profile)
            .map_err(GenerationStartError::Process)?;
        binding
            .verify_image()
            .map_err(|_| GenerationStartError::Unsupported)?;
        let key = generation_key(generation);
        let (mut record, new) = self
            .registry
            .reserve(&key, profile, &binding.digest, self.controller.boot_id())
            .map_err(|_| GenerationStartError::Process(ProcessFault::Unavailable))?;
        self.checked_record(&record)
            .map_err(GenerationStartError::Process)?;
        if new {
            binding
                .claim_user_data(self.registry.identity())
                .map_err(|_| GenerationStartError::Process(ProcessFault::ProfileNotApproved))?;
            record.dispatched = true;
            self.registry
                .save(&record)
                .map_err(|_| GenerationStartError::Process(ProcessFault::Unavailable))?;
            self.controller
                .start(&record, &binding)
                .map_err(GenerationStartError::Process)?;
        }
        match self
            .recovered(record, generation.instance_id())
            .map_err(GenerationStartError::Process)?
        {
            GenerationRecovery::Found(identity) => Ok(ProcessLaunch::new(identity)),
            GenerationRecovery::Indeterminate => {
                Err(GenerationStartError::Process(ProcessFault::Unavailable))
            }
        }
    }

    fn recover_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        profile: LaunchProfile,
    ) -> Result<GenerationRecovery, ProcessFault> {
        self.profile(profile)?;
        let Some(record) = self
            .registry
            .by_key(&generation_key(generation))
            .map_err(|_| ProcessFault::Unavailable)?
        else {
            return Ok(GenerationRecovery::Indeterminate);
        };
        if record.profile != profile {
            return Err(ProcessFault::IdentityMismatch);
        }
        self.recovered(record, generation.instance_id())
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        self.current(process)?
            .0
            .identity
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        let (record, members) = self.current(process)?;
        if members.is_empty() && !record.absent {
            return Err(ProcessFault::InspectionFailed);
        }
        let identity = record.identity.ok_or(ProcessFault::InspectionFailed)?;
        if members
            .iter()
            .any(|member| member.pid == identity.pid() && member.birth == identity.birth_id())
        {
            Ok(ProcessState::Running)
        } else {
            Ok(ProcessState::Exited { code: None })
        }
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        let (record, members) = self.current(process)?;
        if members.is_empty() && !record.absent {
            return Err(ProcessFault::InspectionFailed);
        }
        let identity = record.identity.ok_or(ProcessFault::InspectionFailed)?;
        Ok(members
            .into_iter()
            .filter(|member| member.pid != identity.pid())
            .map(|member| {
                ProcessDescendantIdentity::new(
                    identity.instance_id(),
                    member.pid,
                    member.birth,
                    record.profile.executable(),
                    record.profile.user_data(),
                )
            })
            .collect())
    }

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        let (mut record, _) = self.current(process)?;
        if record.absent {
            return Ok(());
        }
        let binding = self.checked_record(&record)?;
        let instance = record
            .identity
            .as_ref()
            .ok_or(ProcessFault::InspectionFailed)?
            .instance_id();
        record.stopping = true;
        self.registry
            .save(&record)
            .map_err(|_| ProcessFault::Unavailable)?;
        self.controller.stop(&record, &binding, mode)?;
        self.observe(&mut record, instance)?;
        if !record.absent {
            return Err(ProcessFault::StopTimedOut);
        }
        Ok(())
    }
}
