// SPDX-License-Identifier: MIT

//! Opt-in Linux systemd ownership; portable lifecycle requests contain no host paths.

#[path = "process_native_command.rs"]
mod command;
#[path = "process_native_config.rs"]
mod configuration;
#[path = "process_native_identity.rs"]
mod identity;
#[path = "process_native_port.rs"]
mod port;
#[path = "process_native_store.rs"]
mod store;
#[path = "process_native_systemd.rs"]
mod systemd;

use crate::{
    GenerationRecovery, GenerationStartError, InstanceId, LaunchProfile, LaunchSpec,
    ProcessDescendantIdentity, ProcessFault, ProcessHandle, ProcessIdentity, ProcessLaunch,
    ProcessOperationGeneration, ProcessPort, ProcessState, StopMode,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Sanitized deployment errors; host paths and manager output are deliberately absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeProcessError {
    Configuration,
    UnsupportedHost,
    Registry,
    Profile,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GenerationRecord {
    key: String,
    handle: u64,
    unit: String,
    boot_id: String,
    profile: LaunchProfile,
    binding_digest: String,
    dispatched: bool,
    stopping: bool,
    absent: bool,
    identity: Option<ProcessIdentity>,
    invocation: Option<String>,
    control_group: Option<String>,
}

#[derive(Clone, Debug)]
struct NativeMember {
    pid: u64,
    birth: u64,
    control_group: String,
    executable: std::path::PathBuf,
    image_digest: String,
}

#[derive(Clone, Debug)]
enum UnitObservation {
    Missing,
    Present {
        invocation: String,
        control_group: String,
        main_pid: u64,
        members: Vec<NativeMember>,
        terminal: bool,
    },
}

trait ServiceController: Send {
    fn boot_id(&self) -> &str;
    fn start(
        &mut self,
        record: &GenerationRecord,
        profile: &configuration::NativeProfile,
    ) -> Result<(), ProcessFault>;
    fn observe(
        &mut self,
        record: &GenerationRecord,
        profile: &configuration::NativeProfile,
    ) -> Result<UnitObservation, ProcessFault>;
    fn stop(
        &mut self,
        record: &GenerationRecord,
        profile: &configuration::NativeProfile,
        mode: StopMode,
    ) -> Result<(), ProcessFault>;
    fn empty_group(&self, path: &str) -> Result<bool, ProcessFault>;
}

/// Gateway-owned systemd user-service adapter with an exclusive durable generation registry.
///
/// Only explicit deployment configuration can construct this adapter. Opening runs bounded
/// manager-inspection helpers but starts no approved service or game process. Unsupported hosts
/// fail closed. Drop preserves services and their recovery
/// records; callers must use the ordinary fenced lifecycle stop path for cleanup.
pub struct SystemdProcessPort {
    inner: port::NativeProcessPort<systemd::SystemdController>,
}

impl SystemdProcessPort {
    /// Validates complete server-owned profile bindings and an accessible supported manager.
    /// No shell, PID adoption or automatic launch is performed. See ADR 0037 for host gates.
    /// Additional owner journals in `protected_paths` must remain outside every writable profile
    /// root; at most sixteen paths are accepted, each with an existing resolvable parent.
    pub fn open(
        configuration_path: &Path,
        approved: &[LaunchProfile],
        max_records: usize,
        protected_paths: &[&Path],
    ) -> Result<Self, NativeProcessError> {
        if protected_paths.len() > 16 {
            return Err(NativeProcessError::Configuration);
        }
        let configuration = configuration::NativeConfiguration::read(configuration_path, approved)?;
        configuration.protect_paths(protected_paths)?;
        let controller = systemd::SystemdController::open(&configuration)?;
        let registry = store::GenerationRegistry::open(&configuration.registry_path, max_records)?;
        Ok(Self {
            inner: port::NativeProcessPort::new(configuration.profiles, controller, registry),
        })
    }
}

impl ProcessPort for SystemdProcessPort {
    fn start(&mut self, _: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }
    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        self.inner.inspect(process)
    }
    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        self.inner.stop(process, mode)
    }
    fn start_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, GenerationStartError> {
        self.inner
            .start_generation(generation, specification, profile)
    }
    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        self.inner.inspect_identity(process)
    }
    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        self.inner.descendants(process)
    }
    fn recover_owned(
        &mut self,
        _: InstanceId,
        _: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }
    fn recover_generation(
        &mut self,
        generation: &ProcessOperationGeneration,
        profile: LaunchProfile,
    ) -> Result<GenerationRecovery, ProcessFault> {
        self.inner.recover_generation(generation, profile)
    }
}

#[cfg(test)]
#[path = "process_native_tests.rs"]
mod tests;
