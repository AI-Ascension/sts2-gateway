// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::{
    ExecutableIdentity, GenerationRecovery, GenerationStartError, InstanceId, LaunchProfile,
    LaunchSpec, ProcessDescendantIdentity, ProcessFault, ProcessHandle, ProcessIdentity,
    ProcessLaunch, ProcessOperationGeneration, ProcessPort, ProcessState, StopMode,
};

#[derive(Clone, Debug)]
struct Entry {
    identity: ProcessIdentity,
    state: ProcessState,
    descendants: Vec<ProcessDescendantIdentity>,
}

/// Deterministic lifecycle-test process port with exact generation recovery.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeProcess {
    next: u64,
    entries: BTreeMap<ProcessHandle, Entry>,
    start_fault: Option<ProcessFault>,
    identity_fault: Option<ProcessFault>,
    inspect_fault: Option<ProcessFault>,
    stop_fault: Option<ProcessFault>,
    wrong_identity: bool,
    wrong_image: bool,
    retain_after_stop: bool,
    descendants_after_stop: Vec<ProcessDescendantIdentity>,
    recover_enabled: bool,
    recover_fault: Option<ProcessFault>,
    starts: usize,
    stops: Vec<(ProcessHandle, StopMode)>,
    stop_fault_after_first: Option<ProcessFault>,
    generations: BTreeMap<ProcessOperationGeneration, ProcessIdentity>,
    generation_starts: Vec<ProcessOperationGeneration>,
    generation_queries: Vec<ProcessOperationGeneration>,
}

impl FakeProcess {
    pub(crate) fn set_start_fault(&mut self, fault: Option<ProcessFault>) {
        self.start_fault = fault;
    }

    pub(crate) fn set_stop_fault(&mut self, fault: Option<ProcessFault>) {
        self.stop_fault = fault;
    }

    pub(crate) fn set_stop_fault_after_first(&mut self, fault: Option<ProcessFault>) {
        self.stop_fault_after_first = fault;
    }

    pub(crate) fn set_wrong_identity(&mut self, value: bool) {
        self.wrong_identity = value;
    }

    pub(crate) fn set_wrong_image(&mut self, value: bool) {
        self.wrong_image = value;
    }

    pub(crate) fn set_identity_fault(&mut self, fault: Option<ProcessFault>) {
        self.identity_fault = fault;
    }

    pub(crate) fn set_retain_after_stop(&mut self, value: bool) {
        self.retain_after_stop = value;
    }

    pub(crate) fn set_recover_enabled(&mut self, value: bool) {
        self.recover_enabled = value;
    }

    pub(crate) fn set_recover_fault(&mut self, fault: Option<ProcessFault>) {
        self.recover_fault = fault;
    }

    pub(crate) fn set_descendants_after_stop(
        &mut self,
        descendants: Vec<ProcessDescendantIdentity>,
    ) {
        self.descendants_after_stop = descendants;
    }

    pub(crate) fn set_descendants(
        &mut self,
        process: ProcessHandle,
        descendants: Vec<ProcessDescendantIdentity>,
    ) {
        if let Some(entry) = self.entries.get_mut(&process) {
            entry.descendants = descendants;
        }
    }

    pub(crate) fn crash(
        &mut self,
        process: ProcessHandle,
        descendants: Vec<ProcessDescendantIdentity>,
    ) {
        if let Some(entry) = self.entries.get_mut(&process) {
            entry.state = ProcessState::Exited { code: Some(17) };
            entry.descendants = descendants;
        }
    }

    pub(crate) fn starts(&self) -> usize {
        self.starts
    }

    pub(crate) fn stop_modes(&self) -> Vec<StopMode> {
        self.stops.iter().map(|(_, mode)| *mode).collect()
    }

    pub(crate) fn generation_queries(&self) -> &[ProcessOperationGeneration] {
        &self.generation_queries
    }

    pub(crate) fn generation_starts(&self) -> &[ProcessOperationGeneration] {
        &self.generation_starts
    }

    fn create_launch(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        if let Some(fault) = self.start_fault {
            return Err(fault);
        }
        self.next = self.next.saturating_add(1);
        self.starts = self.starts.saturating_add(1);
        let process = ProcessHandle::new(self.next);
        let instance_id = if self.wrong_identity {
            InstanceId::new(specification.instance_id().value().saturating_add(100))
        } else {
            specification.instance_id()
        };
        let executable = if self.wrong_identity || self.wrong_image {
            ExecutableIdentity::new(
                profile.executable().install_id(),
                profile.executable().executable_id(),
                profile.executable().image_id().saturating_add(100),
            )
        } else {
            profile.executable()
        };
        let identity = ProcessIdentity::new(
            instance_id,
            process,
            process.value().saturating_add(10_000),
            process.value().saturating_add(20_000),
            executable,
            profile.user_data(),
        );
        self.entries.insert(
            process,
            Entry {
                identity: identity.clone(),
                state: ProcessState::Running,
                descendants: Vec::new(),
            },
        );
        Ok(ProcessLaunch::new(identity))
    }
}

impl ProcessPort for FakeProcess {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        if let Some(fault) = self.inspect_fault {
            return Err(fault);
        }
        self.entries
            .get(&process)
            .map(|entry| entry.state)
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        if let Some(fault) = self.stop_fault {
            return Err(fault);
        }
        if !self.stops.is_empty()
            && let Some(fault) = self.stop_fault_after_first
        {
            return Err(fault);
        }
        self.stops.push((process, mode));
        if self.retain_after_stop {
            let descendants = self.descendants_after_stop.clone();
            let Some(entry) = self.entries.get_mut(&process) else {
                return Err(ProcessFault::InspectionFailed);
            };
            entry.state = ProcessState::Exited { code: None };
            entry.descendants = descendants;
        } else {
            self.entries.remove(&process);
        }
        Ok(())
    }

    fn start_with_profile(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        self.create_launch(specification, profile)
    }

    fn start_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, GenerationStartError> {
        let launch = self
            .create_launch(specification, profile)
            .map_err(GenerationStartError::Process)?;
        let identity = launch.identity().clone();
        self.generations.insert(generation.clone(), identity);
        self.generation_starts.push(generation.clone());
        Ok(launch)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        if let Some(fault) = self.identity_fault {
            return Err(fault);
        }
        self.entries
            .get(&process)
            .map(|entry| entry.identity.clone())
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        match self.entries.get(&process) {
            Some(entry) => Ok(entry.descendants.clone()),
            None => Ok(Vec::new()),
        }
    }

    fn recover_owned(
        &mut self,
        instance_id: InstanceId,
        profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        if let Some(fault) = self.recover_fault {
            return Err(fault);
        }
        if !self.recover_enabled {
            return Ok(None);
        }
        Ok(self.entries.values().find_map(|entry| {
            (entry.state == ProcessState::Running
                && entry.identity.matches_profile(instance_id, profile))
            .then(|| entry.identity.clone())
        }))
    }

    fn recover_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        _profile: LaunchProfile,
    ) -> Result<GenerationRecovery, ProcessFault> {
        self.generation_queries.push(generation.clone());
        if let Some(fault) = self.recover_fault {
            return Err(fault);
        }
        if !self.recover_enabled {
            return Ok(GenerationRecovery::Indeterminate);
        }
        Ok(self
            .generations
            .get(generation)
            .cloned()
            .filter(|identity| {
                self.entries
                    .get(&identity.process())
                    .is_some_and(|entry| entry.state == ProcessState::Running)
            })
            .map_or(GenerationRecovery::Indeterminate, GenerationRecovery::Found))
    }
}
