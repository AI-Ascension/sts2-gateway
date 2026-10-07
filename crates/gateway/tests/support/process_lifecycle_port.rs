// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use sts2_gateway::{
    ExecutableIdentity, GenerationRecovery, GenerationStartError, InstanceId, LaunchProfile,
    ProcessDescendantIdentity, ProcessFault, ProcessHandle, ProcessIdentity, ProcessLaunch,
    ProcessOperationGeneration, ProcessPort, ProcessState, StopMode,
};

struct Entry {
    identity: ProcessIdentity,
    state: ProcessState,
    descendants: Vec<ProcessDescendantIdentity>,
}

#[derive(Default)]
struct ProcessStateStore {
    next_handle: u64,
    entries: BTreeMap<ProcessHandle, Entry>,
    starts: usize,
    stops: usize,
    fail_stop: bool,
    wrong_image: bool,
    recover: bool,
    descendants_after_stop: Vec<ProcessDescendantIdentity>,
    generations: BTreeMap<ProcessOperationGeneration, ProcessIdentity>,
}

#[derive(Clone, Default)]
pub(super) struct RecordingProcess(Rc<RefCell<ProcessStateStore>>);

impl RecordingProcess {
    pub(super) fn starts(&self) -> usize {
        self.0.borrow().starts
    }

    pub(super) fn stops(&self) -> usize {
        self.0.borrow().stops
    }

    pub(super) fn set_fail_stop(&self, value: bool) {
        self.0.borrow_mut().fail_stop = value;
    }

    pub(super) fn set_wrong_image(&self, value: bool) {
        self.0.borrow_mut().wrong_image = value;
    }

    pub(super) fn set_recover(&self, value: bool) {
        self.0.borrow_mut().recover = value;
    }

    pub(super) fn set_descendants_after_stop(&self, descendants: Vec<ProcessDescendantIdentity>) {
        self.0.borrow_mut().descendants_after_stop = descendants;
    }

    pub(super) fn crash(&self, process: ProcessHandle) {
        if let Some(entry) = self.0.borrow_mut().entries.get_mut(&process) {
            entry.state = ProcessState::Exited { code: Some(17) };
        }
    }

    fn create_launch(
        &mut self,
        specification: sts2_gateway::LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        let mut state = self.0.borrow_mut();
        state.next_handle = state.next_handle.saturating_add(1);
        state.starts += 1;
        let handle = ProcessHandle::new(state.next_handle);
        let executable = if state.wrong_image {
            ExecutableIdentity::new(91, 92, 93)
        } else {
            profile.executable()
        };
        let identity = ProcessIdentity::new(
            specification.instance_id(),
            handle,
            1000 + handle.value(),
            5000 + handle.value(),
            executable,
            profile.user_data(),
        );
        state.entries.insert(
            handle,
            Entry {
                identity: identity.clone(),
                state: ProcessState::Running,
                descendants: Vec::new(),
            },
        );
        Ok(ProcessLaunch::new(identity))
    }
}

impl ProcessPort for RecordingProcess {
    fn start(
        &mut self,
        _specification: sts2_gateway::LaunchSpec,
    ) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn start_with_profile(
        &mut self,
        specification: sts2_gateway::LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        self.create_launch(specification, profile)
    }

    fn start_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        specification: sts2_gateway::LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, GenerationStartError> {
        if generation.instance_id() != specification.instance_id()
            || generation.profile_id() != profile.id()
        {
            return Err(GenerationStartError::Process(
                ProcessFault::ProfileNotApproved,
            ));
        }
        let launch = self
            .create_launch(specification, profile)
            .map_err(GenerationStartError::Process)?;
        self.0
            .borrow_mut()
            .generations
            .insert(generation.clone(), launch.identity().clone());
        Ok(launch)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.state)
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.identity.clone())
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn stop(&mut self, process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        let mut state = self.0.borrow_mut();
        state.stops += 1;
        if state.fail_stop {
            return Err(ProcessFault::StopFailed);
        }
        let survivors = state.descendants_after_stop.clone();
        let Some(entry) = state.entries.get_mut(&process) else {
            return Err(ProcessFault::InspectionFailed);
        };
        entry.state = ProcessState::Exited { code: None };
        entry.descendants = survivors;
        Ok(())
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        self.0
            .borrow()
            .entries
            .get(&process)
            .map(|entry| entry.descendants.clone())
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn recover_owned(
        &mut self,
        instance_id: InstanceId,
        profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        let state = self.0.borrow();
        if !state.recover {
            return Ok(None);
        }
        Ok(state
            .entries
            .values()
            .find(|entry| {
                entry.state == ProcessState::Running
                    && entry.identity.matches_profile(instance_id, profile)
            })
            .map(|entry| entry.identity.clone()))
    }

    fn recover_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        _profile: LaunchProfile,
    ) -> Result<GenerationRecovery, ProcessFault> {
        let state = self.0.borrow();
        if !state.recover {
            return Ok(GenerationRecovery::Indeterminate);
        }
        Ok(state
            .generations
            .get(generation)
            .cloned()
            .filter(|identity| {
                state
                    .entries
                    .get(&identity.process())
                    .is_some_and(|entry| entry.state == ProcessState::Running)
            })
            .map_or(GenerationRecovery::Indeterminate, GenerationRecovery::Found))
    }
}
