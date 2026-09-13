// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::{
    InstanceId, LaunchProfileId, LaunchSpec, ProcessFault, ProcessHandle, ProcessIdentity,
    ProcessPort, ProcessState, StopMode,
};

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
    IdentityMismatch,
    ForeignDescendant,
    UserDataNamespaceBusy,
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
    identities: BTreeMap<InstanceId, ProcessIdentity>,
}

impl<P: ProcessPort> ProcessSupervisor<P> {
    pub fn new(config: ProcessSupervisorConfig, process: P) -> Self {
        Self {
            config,
            process,
            owned: BTreeMap::new(),
            identities: BTreeMap::new(),
        }
    }

    pub fn start(
        &mut self,
        instance_id: InstanceId,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        self.start_spec(LaunchSpec::new(instance_id))
    }

    pub fn start_with_profile(
        &mut self,
        instance_id: InstanceId,
        profile_id: LaunchProfileId,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        self.start_spec(LaunchSpec::for_profile(instance_id, profile_id))
    }

    pub fn start_spec(
        &mut self,
        specification: LaunchSpec,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        let instance_id = specification.instance_id();
        if self.owned.contains_key(&instance_id) {
            return Err(ProcessSupervisorError::AlreadyOwned);
        }
        if self.owned.len() >= self.config.max_owned_processes() {
            return Err(ProcessSupervisorError::CapacityExceeded);
        }
        let handle = self
            .process
            .start(specification)
            .map_err(ProcessSupervisorError::Process)?;
        self.identities.remove(&instance_id);
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

    /// Records an identity restored from a durable gateway operation.
    ///
    /// The identity is required before `attach_authorized`; a caller cannot adopt a process by
    /// supplying an identity that the supervisor has never authorized.
    pub fn authorize_identity(
        &mut self,
        identity: ProcessIdentity,
    ) -> Result<(), ProcessSupervisorError> {
        if identity.instance_id().value() == 0
            || identity.process().value() == 0
            || identity.pid() == 0
            || identity.birth_id() == 0
            || identity.executable().is_none()
            || identity.user_data().is_none()
        {
            return Err(ProcessSupervisorError::IdentityMismatch);
        }
        if self.owned.contains_key(&identity.instance_id()) {
            return Err(ProcessSupervisorError::AlreadyOwned);
        }
        if self.namespace_is_reserved_by_other(
            identity.instance_id(),
            identity
                .user_data()
                .ok_or(ProcessSupervisorError::IdentityMismatch)?,
        ) {
            return Err(ProcessSupervisorError::UserDataNamespaceBusy);
        }
        self.identities.insert(identity.instance_id(), identity);
        Ok(())
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
        self.identities.remove(&instance_id);
        self.owned.remove(&instance_id);
        Ok(())
    }

    /// Replaces one owned process with a fresh handle after a controlled stop.
    ///
    /// The old handle is removed before the replacement is started. If the replacement cannot
    /// start, the instance remains unowned and callers must fail closed or allocate anew.
    pub fn restart(
        &mut self,
        instance_id: InstanceId,
    ) -> Result<ProcessHandle, ProcessSupervisorError> {
        let old_handle = self
            .owned
            .get(&instance_id)
            .copied()
            .ok_or(ProcessSupervisorError::NotOwned)?;
        self.process
            .stop(old_handle, StopMode::Force)
            .map_err(ProcessSupervisorError::Process)?;
        self.identities.remove(&instance_id);
        self.owned.remove(&instance_id);
        let new_handle = self
            .process
            .start(crate::LaunchSpec::new(instance_id))
            .map_err(ProcessSupervisorError::Process)?;
        self.identities.remove(&instance_id);
        self.owned.insert(instance_id, new_handle);
        Ok(new_handle)
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

    /// Returns whether a user-data namespace is already held by another
    /// instance. Legacy owned handles have no namespace identity, so they are
    /// treated as conflicting: the supervisor cannot prove that a new
    /// profile launch would be isolated from them.
    pub(crate) fn namespace_is_reserved_by_other(
        &self,
        instance_id: InstanceId,
        namespace: crate::UserDataConfig,
    ) -> bool {
        if self.identities.iter().any(|(owned_instance, identity)| {
            *owned_instance != instance_id && identity.user_data() == Some(namespace)
        }) {
            return true;
        }
        self.owned.keys().any(|owned_instance| {
            if *owned_instance == instance_id {
                return false;
            }
            self.identities
                .get(owned_instance)
                .and_then(ProcessIdentity::user_data)
                .is_none()
        })
    }
}

#[path = "process_supervisor_resolved.rs"]
mod process_supervisor_resolved;
