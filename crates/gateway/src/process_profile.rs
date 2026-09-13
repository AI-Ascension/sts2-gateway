// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Opaque identifier for a server-approved launch profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LaunchProfileId(u64);

impl LaunchProfileId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn try_new(value: u64) -> Result<Self, LaunchProfileError> {
        if value == 0 {
            return Err(LaunchProfileError::InvalidId);
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Exact executable/install identity resolved by the gateway, never supplied by a caller.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentity {
    install_id: u64,
    executable_id: u64,
    image_id: u64,
}

impl ExecutableIdentity {
    pub const fn new(install_id: u64, executable_id: u64, image_id: u64) -> Self {
        Self {
            install_id,
            executable_id,
            image_id,
        }
    }

    pub const fn try_new(
        install_id: u64,
        executable_id: u64,
        image_id: u64,
    ) -> Result<Self, LaunchProfileError> {
        if install_id == 0 || executable_id == 0 || image_id == 0 {
            return Err(LaunchProfileError::InvalidExecutableIdentity);
        }
        Ok(Self {
            install_id,
            executable_id,
            image_id,
        })
    }

    pub const fn install_id(self) -> u64 {
        self.install_id
    }

    pub const fn executable_id(self) -> u64 {
        self.executable_id
    }

    pub const fn image_id(self) -> u64 {
        self.image_id
    }

    const fn is_valid(self) -> bool {
        self.install_id != 0 && self.executable_id != 0 && self.image_id != 0
    }
}

/// Server-owned namespace selecting isolated user data for a process.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserDataConfig {
    namespace_id: u64,
}

impl UserDataConfig {
    pub const fn new(namespace_id: u64) -> Self {
        Self { namespace_id }
    }

    pub const fn try_new(namespace_id: u64) -> Result<Self, LaunchProfileError> {
        if namespace_id == 0 {
            return Err(LaunchProfileError::InvalidUserData);
        }
        Ok(Self { namespace_id })
    }

    pub const fn namespace_id(self) -> u64 {
        self.namespace_id
    }

    const fn is_valid(self) -> bool {
        self.namespace_id != 0
    }
}

pub const MAX_PROFILE_DESCENDANTS: usize = 64;
pub const MAX_START_TIMEOUT_MILLIS: u64 = 60_000;
pub const MAX_STOP_TIMEOUT_MILLIS: u64 = 60_000;

/// Bounded process behavior approved together with a launch profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessPolicy {
    max_descendants: usize,
    start_timeout_millis: u64,
    stop_timeout_millis: u64,
}

impl ProcessPolicy {
    pub const fn try_new(
        max_descendants: usize,
        start_timeout_millis: u64,
        stop_timeout_millis: u64,
    ) -> Result<Self, LaunchProfileError> {
        if max_descendants == 0 || max_descendants > MAX_PROFILE_DESCENDANTS {
            return Err(LaunchProfileError::InvalidPolicy);
        }
        if start_timeout_millis == 0 || start_timeout_millis > MAX_START_TIMEOUT_MILLIS {
            return Err(LaunchProfileError::InvalidPolicy);
        }
        if stop_timeout_millis == 0 || stop_timeout_millis > MAX_STOP_TIMEOUT_MILLIS {
            return Err(LaunchProfileError::InvalidPolicy);
        }
        Ok(Self {
            max_descendants,
            start_timeout_millis,
            stop_timeout_millis,
        })
    }

    pub const fn max_descendants(self) -> usize {
        self.max_descendants
    }

    pub const fn start_timeout_millis(self) -> u64 {
        self.start_timeout_millis
    }

    pub const fn stop_timeout_millis(self) -> u64 {
        self.stop_timeout_millis
    }

    const fn is_valid(self) -> bool {
        self.max_descendants > 0
            && self.max_descendants <= MAX_PROFILE_DESCENDANTS
            && self.start_timeout_millis > 0
            && self.start_timeout_millis <= MAX_START_TIMEOUT_MILLIS
            && self.stop_timeout_millis > 0
            && self.stop_timeout_millis <= MAX_STOP_TIMEOUT_MILLIS
    }
}

/// A complete launch description obtained only after resolving an opaque profile ID.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProfile {
    id: LaunchProfileId,
    executable: ExecutableIdentity,
    user_data: UserDataConfig,
    policy: ProcessPolicy,
}

impl LaunchProfile {
    pub const fn try_new(
        id: LaunchProfileId,
        executable: ExecutableIdentity,
        user_data: UserDataConfig,
        policy: ProcessPolicy,
    ) -> Result<Self, LaunchProfileError> {
        if id.value() == 0 {
            return Err(LaunchProfileError::InvalidId);
        }
        if !executable.is_valid() {
            return Err(LaunchProfileError::InvalidExecutableIdentity);
        }
        if !user_data.is_valid() {
            return Err(LaunchProfileError::InvalidUserData);
        }
        if !policy.is_valid() {
            return Err(LaunchProfileError::InvalidPolicy);
        }
        Ok(Self {
            id,
            executable,
            user_data,
            policy,
        })
    }

    pub const fn id(self) -> LaunchProfileId {
        self.id
    }

    pub const fn executable(self) -> ExecutableIdentity {
        self.executable
    }

    pub const fn user_data(self) -> UserDataConfig {
        self.user_data
    }

    pub const fn policy(self) -> ProcessPolicy {
        self.policy
    }
}

/// Errors raised while constructing or resolving server-owned launch profiles.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LaunchProfileError {
    InvalidId,
    DuplicateId,
    NotFound,
    InvalidExecutableIdentity,
    InvalidUserData,
    InvalidPolicy,
    CapacityExceeded,
}

/// Bounded server-side catalog; callers receive only an opaque profile ID.
#[derive(Clone, Debug)]
pub struct ApprovedLaunchProfiles {
    profiles: BTreeMap<LaunchProfileId, LaunchProfile>,
    capacity: usize,
}

impl Default for ApprovedLaunchProfiles {
    fn default() -> Self {
        Self::new(64)
    }
}

impl ApprovedLaunchProfiles {
    pub const fn new(capacity: usize) -> Self {
        Self {
            profiles: BTreeMap::new(),
            capacity,
        }
    }

    pub const fn try_new(capacity: usize) -> Result<Self, LaunchProfileError> {
        if capacity == 0 {
            return Err(LaunchProfileError::CapacityExceeded);
        }
        Ok(Self::new(capacity))
    }

    pub fn insert(&mut self, profile: LaunchProfile) -> Result<(), LaunchProfileError> {
        if profile.id().value() == 0 {
            return Err(LaunchProfileError::InvalidId);
        }
        if !profile.executable().is_valid() {
            return Err(LaunchProfileError::InvalidExecutableIdentity);
        }
        if !profile.user_data().is_valid() {
            return Err(LaunchProfileError::InvalidUserData);
        }
        if !profile.policy().is_valid() {
            return Err(LaunchProfileError::InvalidPolicy);
        }
        if self.profiles.len() >= self.capacity && !self.profiles.contains_key(&profile.id()) {
            return Err(LaunchProfileError::CapacityExceeded);
        }
        if self.profiles.contains_key(&profile.id()) {
            return Err(LaunchProfileError::DuplicateId);
        }
        self.profiles.insert(profile.id(), profile);
        Ok(())
    }

    pub fn resolve(&self, id: LaunchProfileId) -> Result<LaunchProfile, LaunchProfileError> {
        self.profiles
            .get(&id)
            .copied()
            .ok_or(LaunchProfileError::NotFound)
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}
