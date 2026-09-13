// SPDX-License-Identifier: MIT

use crate::process_profile::{ExecutableIdentity, LaunchProfile, UserDataConfig};
use crate::{InstanceId, ProcessHandle};
use serde::{Deserialize, Serialize};

/// Identity captured when a process is created or reattached.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    instance_id: InstanceId,
    process: ProcessHandle,
    pid: u64,
    birth_id: u64,
    executable: Option<ExecutableIdentity>,
    user_data: Option<UserDataConfig>,
}

impl ProcessIdentity {
    pub const fn new(
        instance_id: InstanceId,
        process: ProcessHandle,
        pid: u64,
        birth_id: u64,
        executable: ExecutableIdentity,
        user_data: UserDataConfig,
    ) -> Self {
        Self {
            instance_id,
            process,
            pid,
            birth_id,
            executable: Some(executable),
            user_data: Some(user_data),
        }
    }

    pub const fn legacy(process: ProcessHandle) -> Self {
        Self {
            instance_id: InstanceId::new(0),
            process,
            pid: process.value(),
            birth_id: 0,
            executable: None,
            user_data: None,
        }
    }

    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn process(&self) -> ProcessHandle {
        self.process
    }

    pub const fn pid(&self) -> u64 {
        self.pid
    }

    pub const fn birth_id(&self) -> u64 {
        self.birth_id
    }

    pub const fn executable(&self) -> Option<ExecutableIdentity> {
        self.executable
    }

    pub const fn user_data(&self) -> Option<UserDataConfig> {
        self.user_data
    }

    pub fn matches_profile(&self, instance_id: InstanceId, profile: LaunchProfile) -> bool {
        self.process.value() != 0
            && self.pid != 0
            && self.birth_id != 0
            && self.instance_id == instance_id
            && self.executable == Some(profile.executable())
            && self.user_data == Some(profile.user_data())
    }
}

/// Identity for one process descendant that remains inside the owned process tree.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessDescendantIdentity {
    instance_id: InstanceId,
    pid: u64,
    birth_id: u64,
    executable: ExecutableIdentity,
    user_data: UserDataConfig,
}

impl ProcessDescendantIdentity {
    pub const fn new(
        instance_id: InstanceId,
        pid: u64,
        birth_id: u64,
        executable: ExecutableIdentity,
        user_data: UserDataConfig,
    ) -> Self {
        Self {
            instance_id,
            pid,
            birth_id,
            executable,
            user_data,
        }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn pid(self) -> u64 {
        self.pid
    }

    pub const fn birth_id(self) -> u64 {
        self.birth_id
    }

    pub const fn executable(self) -> ExecutableIdentity {
        self.executable
    }

    pub const fn user_data(self) -> UserDataConfig {
        self.user_data
    }

    pub fn matches_profile(self, instance_id: InstanceId, profile: LaunchProfile) -> bool {
        self.pid != 0
            && self.birth_id != 0
            && self.instance_id == instance_id
            && self.executable == profile.executable()
            && self.user_data == profile.user_data()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLaunch {
    identity: ProcessIdentity,
}

impl ProcessLaunch {
    pub const fn new(identity: ProcessIdentity) -> Self {
        Self { identity }
    }

    pub const fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }
}
