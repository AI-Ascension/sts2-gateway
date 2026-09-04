// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::{InstanceId, ProcessFault, ProcessHandle, ProcessPort, ProcessState, StopMode};

/// Bounds the number of processes owned by one gateway supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessSupervisorConfig {
    max_owned_processes: usize,
}

impl ProcessSupervisorConfig {
    pub const fn new(max_owned_processes: usize) -> Self {
        Self {
            max_owned_processes,
        }
    }

    pub const fn try_new(max_owned_processes: usize) -> Result<Self, ProcessSupervisorConfigError> {
        if max_owned_processes == 0 {
            return Err(ProcessSupervisorConfigError::ZeroCapacity);
        }
        Ok(Self::new(max_owned_processes))
    }

    pub const fn max_owned_processes(self) -> usize {
        self.max_owned_processes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessSupervisorConfigError {
    ZeroCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessSupervisorError {
    CapacityExceeded,
    AlreadyOwned,
    NotOwned,
    Process(ProcessFault),
}

/// Owns only the process-port handles assigned to this supervisor.
///
/// The supervisor deliberately accepts an injected `ProcessPort`; it does not
/// know executable paths, environment variables, ports, game files, or host
/// objects. Concrete launch policy belongs to the deployment that implements
/// that port.
pub struct ProcessSupervisor<P> {
    config: ProcessSupervisorConfig,
    process: P,
    owned: BTreeMap<InstanceId, ProcessHandle>,
}

impl<P: ProcessPort> ProcessSupervisor<P> {
    pub fn new(config: ProcessSupervisorConfig, process: P) -> Self {
        Self {
            config,
            process,
            owned: BTreeMap::new(),
        }
    }

    pub fn start(
        &mut self,
        instance_id: InstanceId,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        if self.owned.contains_key(&instance_id) {
            return Err(ProcessSupervisorError::AlreadyOwned);
        }
        if self.owned.len() >= self.config.max_owned_processes() {
            return Err(ProcessSupervisorError::CapacityExceeded);
        }
        let handle = self
            .process
            .start(crate::LaunchSpec::new(instance_id))
            .map_err(ProcessSupervisorError::Process)?;
        self.owned.insert(instance_id, handle);
        Ok(handle)
    }

    pub fn inspect(
        &mut self,
        instance_id: InstanceId,
    ) -> Result<ProcessState, ProcessSupervisorError> {
        let handle = self
            .owned
            .get(&instance_id)
            .copied()
            .ok_or(ProcessSupervisorError::NotOwned)?;
        self.process
            .inspect(handle)
            .map_err(ProcessSupervisorError::Process)
    }

    pub fn stop(
        &mut self,
        instance_id: InstanceId,
        mode: StopMode,
    ) -> Result<(), ProcessSupervisorError> {
        let handle = self
            .owned
            .get(&instance_id)
            .copied()
            .ok_or(ProcessSupervisorError::NotOwned)?;
        self.process
            .stop(handle, mode)
            .map_err(ProcessSupervisorError::Process)?;
        self.owned.remove(&instance_id);
        Ok(())
    }

    pub fn process_handle(&self, instance_id: InstanceId) -> Option<ProcessHandle> {
        self.owned.get(&instance_id).copied()
    }

    pub fn owned_count(&self) -> usize {
        self.owned.len()
    }

    pub fn is_owned(&self, instance_id: InstanceId) -> bool {
        self.owned.contains_key(&instance_id)
    }

    pub fn config(&self) -> ProcessSupervisorConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ProcessSupervisor, ProcessSupervisorConfig, ProcessSupervisorError};
    use crate::{
        InstanceId, LaunchSpec, ProcessFault, ProcessHandle, ProcessPort, ProcessState, StopMode,
    };

    #[derive(Default)]
    struct FakeProcess {
        next: u64,
        states: BTreeMap<u64, ProcessState>,
    }

    impl ProcessPort for FakeProcess {
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
    fn owns_bounded_handles_and_releases_only_after_stop() -> Result<(), String> {
        let config = ProcessSupervisorConfig::try_new(1).map_err(|error| format!("{error:?}"))?;
        let mut supervisor = ProcessSupervisor::new(config, FakeProcess::default());
        let first = supervisor
            .start(InstanceId::new(1))
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(supervisor.process_handle(InstanceId::new(1)), Some(first));
        assert_eq!(
            supervisor.start(InstanceId::new(2)),
            Err(ProcessSupervisorError::CapacityExceeded)
        );
        supervisor
            .stop(InstanceId::new(1), StopMode::Graceful)
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(supervisor.owned_count(), 0);
        Ok(())
    }
}
