// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use sts2_gateway::{
    ExecutableIdentity, InstanceId, LaunchProfile, LaunchProfileId, LaunchSpec, ProcessFault,
    ProcessHandle, ProcessIdentity, ProcessLaunch, ProcessPolicy, ProcessPort, ProcessState,
    ProcessSupervisor, ProcessSupervisorConfig, ProcessSupervisorError, StopMode, UserDataConfig,
};

#[derive(Default)]
struct FakeProcessPort {
    next: u64,
    states: BTreeMap<u64, ProcessState>,
    fail_stop: bool,
    fail_after_first_start: bool,
}

impl ProcessPort for FakeProcessPort {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        if self.fail_after_first_start && self.next > 0 {
            return Err(ProcessFault::StartRejected);
        }
        self.next = self.next.saturating_add(1);
        let handle = ProcessHandle::new(self.next);
        self.states.insert(handle.value(), ProcessState::Running);
        Ok(handle)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.states
            .get(&process.value())
            .copied()
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn stop(&mut self, process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        if self.fail_stop {
            return Err(ProcessFault::StopFailed);
        }
        self.states.remove(&process.value());
        Ok(())
    }
}

#[test]
fn restart_retains_old_ownership_on_stop_failure_and_none_on_start_failure() -> Result<(), String> {
    let instance = InstanceId::new(1);
    let config = ProcessSupervisorConfig::try_new(1).map_err(|error| format!("{error:?}"))?;
    let mut stopping = ProcessSupervisor::new(
        config,
        FakeProcessPort {
            fail_stop: true,
            ..FakeProcessPort::default()
        },
    );
    let old = stopping
        .start(instance)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        stopping.restart(instance),
        Err(ProcessSupervisorError::Process(ProcessFault::StopFailed))
    );
    assert_eq!(stopping.process_handle(instance), Some(old));
    let mut starting = ProcessSupervisor::new(
        config,
        FakeProcessPort {
            fail_after_first_start: true,
            ..FakeProcessPort::default()
        },
    );
    starting
        .start(instance)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        starting.restart(instance),
        Err(ProcessSupervisorError::Process(ProcessFault::StartRejected))
    );
    assert!(!starting.is_owned(instance));
    assert_eq!(starting.owned_count(), 0);
    Ok(())
}

#[test]
fn capacity_and_ownership_are_enforced_before_process_port_calls() -> Result<(), String> {
    let config = ProcessSupervisorConfig::try_new(1).map_err(|error| format!("{error:?}"))?;
    let mut supervisor = ProcessSupervisor::new(config, FakeProcessPort::default());
    supervisor
        .start(InstanceId::new(1))
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        supervisor.start(InstanceId::new(2)),
        Err(ProcessSupervisorError::CapacityExceeded)
    );
    assert_eq!(
        supervisor.start(InstanceId::new(1)),
        Err(ProcessSupervisorError::AlreadyOwned)
    );
    assert_eq!(
        supervisor.inspect(InstanceId::new(1)),
        Ok(ProcessState::Running)
    );
    supervisor
        .stop(InstanceId::new(1), StopMode::Force)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        supervisor.inspect(InstanceId::new(1)),
        Err(ProcessSupervisorError::NotOwned)
    );
    Ok(())
}

#[test]
fn attach_rejects_an_identity_without_a_durable_authorization() -> Result<(), String> {
    let config = ProcessSupervisorConfig::try_new(1).map_err(|error| format!("{error:?}"))?;
    let mut supervisor = ProcessSupervisor::new(config, FakeProcessPort::default());
    let identity = ProcessIdentity::new(
        InstanceId::new(1),
        ProcessHandle::new(10),
        11,
        12,
        ExecutableIdentity::new(21, 22, 23),
        UserDataConfig::new(24),
    );
    assert_eq!(
        supervisor.attach_authorized(identity),
        Err(ProcessSupervisorError::IdentityMismatch)
    );
    Ok(())
}

#[derive(Default)]
struct ResolvedProcessPort {
    next: u64,
    states: BTreeMap<ProcessHandle, ProcessState>,
    identities: BTreeMap<ProcessHandle, ProcessIdentity>,
}

impl ProcessPort for ResolvedProcessPort {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn start_with_profile(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        self.next = self.next.saturating_add(1);
        let process = ProcessHandle::new(self.next);
        let identity = ProcessIdentity::new(
            specification.instance_id(),
            process,
            process.value().saturating_add(10),
            process.value().saturating_add(20),
            profile.executable(),
            profile.user_data(),
        );
        self.states.insert(process, ProcessState::Running);
        self.identities.insert(process, identity.clone());
        Ok(ProcessLaunch::new(identity))
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.states
            .get(&process)
            .copied()
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        self.identities
            .get(&process)
            .cloned()
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn descendants(
        &mut self,
        _process: ProcessHandle,
    ) -> Result<Vec<sts2_gateway::ProcessDescendantIdentity>, ProcessFault> {
        Ok(Vec::new())
    }

    fn stop(&mut self, process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        self.states
            .insert(process, ProcessState::Exited { code: None });
        Ok(())
    }
}

fn resolved_profile() -> Result<LaunchProfile, String> {
    let executable =
        ExecutableIdentity::try_new(31, 32, 33).map_err(|error| format!("{error:?}"))?;
    let user_data = UserDataConfig::try_new(34).map_err(|error| format!("{error:?}"))?;
    let policy = ProcessPolicy::try_new(2, 1_000, 1_000).map_err(|error| format!("{error:?}"))?;
    LaunchProfile::try_new(LaunchProfileId::new(1), executable, user_data, policy)
        .map_err(|error| format!("{error:?}"))
}

#[test]
fn resolved_supervisor_restarts_only_the_verified_profile_identity() -> Result<(), String> {
    let instance = InstanceId::new(9);
    let profile = resolved_profile()?;
    let config = ProcessSupervisorConfig::try_new(1).map_err(|error| format!("{error:?}"))?;
    let mut supervisor = ProcessSupervisor::new(config, ResolvedProcessPort::default());
    let first = supervisor
        .start_resolved(LaunchSpec::for_profile(instance, profile.id()), profile)
        .map_err(|error| format!("{error:?}"))?;
    let replacement = supervisor
        .restart_resolved(instance, profile)
        .map_err(|error| format!("{error:?}"))?;
    assert_ne!(first.process(), replacement.process());
    assert_eq!(
        supervisor.process_handle(instance),
        Some(replacement.process())
    );
    Ok(())
}
