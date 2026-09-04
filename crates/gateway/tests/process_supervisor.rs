// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use sts2_gateway::{
    InstanceId, LaunchSpec, ProcessFault, ProcessHandle, ProcessPort, ProcessState,
    ProcessSupervisor, ProcessSupervisorConfig, ProcessSupervisorError, StopMode,
};

#[derive(Default)]
struct FakeProcessPort {
    next: u64,
    states: BTreeMap<u64, ProcessState>,
}

impl ProcessPort for FakeProcessPort {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
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
        self.states.remove(&process.value());
        Ok(())
    }
}

#[test]
fn capacity_and_ownership_are_enforced_before_process_port_calls() {
    let config = ProcessSupervisorConfig::try_new(1).expect("valid process supervisor config");
    let mut supervisor = ProcessSupervisor::new(config, FakeProcessPort::default());
    supervisor
        .start(InstanceId::new(1))
        .expect("first process starts");
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
        .expect("owned process stops");
    assert_eq!(
        supervisor.inspect(InstanceId::new(1)),
        Err(ProcessSupervisorError::NotOwned)
    );
}
