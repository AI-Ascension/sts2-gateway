// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

/// Proposed gateway-local contract until launch-profile issue #50 is accepted by all owners.
pub const LAUNCH_PROFILE_CONTRACT: &str = "gateway-launch-profile-v1";
/// The only launch profile this source/component slice may bind for disposable automation.
pub const LAUNCH_PROFILE_ID: &str = "automation-disposable-v1";
/// Gateway-local profile operation contract.  It is not a shared protocol artifact yet.
pub const SAVE_PROFILE_CONTRACT: &str = "gateway-save-profile-v1";
pub const SAVE_PROFILE_MAX_BODY_BYTES: usize = 16 * 1024;
pub const SAVE_PROFILE_MAX_IDENTITY_BYTES: usize = 128;
pub const SAVE_PROFILE_MAX_OPERATION_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SaveProfileId(String);

impl SaveProfileId {
    pub fn try_new(value: impl Into<String>) -> Result<Self, SaveProfileIdError> {
        let value = value.into();
        if !valid_identity(&value, SAVE_PROFILE_MAX_IDENTITY_BYTES) {
            return Err(SaveProfileIdError::Invalid);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileIdError {
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserDataIdentity(u64);

impl UserDataIdentity {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn try_new(value: u64) -> Result<Self, UserDataIdentityError> {
        if value == 0 {
            return Err(UserDataIdentityError::Invalid);
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataIdentityError {
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBaseline {
    pub identity: String,
    pub digest: String,
}

impl ProfileBaseline {
    pub fn try_new(
        identity: impl Into<String>,
        digest: impl Into<String>,
    ) -> Result<Self, ProfileBaselineError> {
        let baseline = Self {
            identity: identity.into(),
            digest: digest.into(),
        };
        baseline.validate()?;
        Ok(baseline)
    }

    pub fn validate(&self) -> Result<(), ProfileBaselineError> {
        if !valid_identity(&self.identity, SAVE_PROFILE_MAX_IDENTITY_BYTES) {
            return Err(ProfileBaselineError::InvalidIdentity);
        }
        if !is_digest(&self.digest) {
            return Err(ProfileBaselineError::InvalidDigest);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileBaselineError {
    InvalidIdentity,
    InvalidDigest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDataProvenance {
    pub owner: String,
    pub instance_id: String,
    pub operation_id: String,
    pub contract: String,
}

impl UserDataProvenance {
    pub fn validate(&self) -> Result<(), UserDataProvenanceError> {
        for value in [
            &self.owner,
            &self.instance_id,
            &self.operation_id,
            &self.contract,
        ] {
            if !valid_identity(value, SAVE_PROFILE_MAX_IDENTITY_BYTES) {
                return Err(UserDataProvenanceError::InvalidIdentity);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserDataDescriptor {
    pub identity: UserDataIdentity,
    pub provenance: UserDataProvenance,
    #[serde(default)]
    pub baseline: Option<ProfileBaseline>,
}

impl UserDataDescriptor {
    pub fn validate(&self) -> Result<(), UserDataDescriptorError> {
        if self.identity.value() == 0 {
            return Err(UserDataDescriptorError::Identity);
        }
        self.provenance
            .validate()
            .map_err(UserDataDescriptorError::Provenance)?;
        if let Some(baseline) = self.baseline.as_ref() {
            baseline
                .validate()
                .map_err(UserDataDescriptorError::Baseline)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataProvenanceError {
    InvalidIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDataDescriptorError {
    Identity,
    Provenance(UserDataProvenanceError),
    Baseline(ProfileBaselineError),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProfileBinding {
    pub contract: String,
    pub profile_id: String,
    pub user_data: UserDataIdentity,
}

impl LaunchProfileBinding {
    pub fn try_new(
        profile_id: impl Into<String>,
        user_data: UserDataIdentity,
    ) -> Result<Self, LaunchProfileBindingError> {
        let binding = Self {
            contract: LAUNCH_PROFILE_CONTRACT.to_owned(),
            profile_id: profile_id.into(),
            user_data,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn validate(&self) -> Result<(), LaunchProfileBindingError> {
        if self.contract != LAUNCH_PROFILE_CONTRACT {
            return Err(LaunchProfileBindingError::Contract);
        }
        if self.user_data.value() == 0 {
            return Err(LaunchProfileBindingError::UserData);
        }
        if !valid_identity(&self.profile_id, SAVE_PROFILE_MAX_IDENTITY_BYTES) {
            return Err(LaunchProfileBindingError::Profile);
        }
        if self.profile_id != LAUNCH_PROFILE_ID {
            return Err(LaunchProfileBindingError::Profile);
        }
        Ok(())
    }
}

/// Port for the not-yet-merged #50 launch-profile owner.
///
/// The port returns an opaque profile binding only.  It must never accept a caller path,
/// command, URL, environment override or user-data root.
pub trait LaunchProfileBindingPort {
    fn bind_disposable(
        &mut self,
        user_data: UserDataIdentity,
    ) -> Result<LaunchProfileBinding, LaunchProfileBindingError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchProfileBindingError {
    Contract,
    Profile,
    UserData,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileContext {
    pub instance_id: String,
    pub caller_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub correlation_id: String,
}

impl SaveProfileContext {
    pub fn validate(&self) -> Result<(), SaveProfileFenceError> {
        for value in [
            &self.instance_id,
            &self.caller_id,
            &self.session_id,
            &self.lease_id,
            &self.correlation_id,
        ] {
            if !valid_identity(value, SAVE_PROFILE_MAX_IDENTITY_BYTES) {
                return Err(SaveProfileFenceError::InvalidIdentity);
            }
        }
        Ok(())
    }

    pub fn same_fence(&self, other: &Self) -> bool {
        self.instance_id == other.instance_id
            && self.caller_id == other.caller_id
            && self.session_id == other.session_id
            && self.lease_id == other.lease_id
            && self.lease_epoch == other.lease_epoch
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileAuthority {
    pub instance_id: String,
    pub caller_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub expires_at_millis: Option<u64>,
}

impl SaveProfileAuthority {
    pub fn authorize(
        &self,
        context: &SaveProfileContext,
        now_millis: u64,
    ) -> Result<(), SaveProfileFenceError> {
        context.validate()?;
        if self
            .expires_at_millis
            .is_some_and(|deadline| now_millis >= deadline)
        {
            return Err(SaveProfileFenceError::Expired);
        }
        if self.instance_id != context.instance_id {
            return Err(SaveProfileFenceError::WrongInstance);
        }
        if self.caller_id != context.caller_id {
            return Err(SaveProfileFenceError::WrongCaller);
        }
        if self.session_id != context.session_id {
            return Err(SaveProfileFenceError::WrongSession);
        }
        if self.lease_id != context.lease_id {
            return Err(SaveProfileFenceError::WrongLease);
        }
        if self.lease_epoch != context.lease_epoch {
            return Err(SaveProfileFenceError::StaleEpoch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileFenceError {
    InvalidIdentity,
    WrongInstance,
    WrongCaller,
    WrongSession,
    WrongLease,
    StaleEpoch,
    Expired,
}

fn valid_identity(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.contains("..")
        && !value.contains("://")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
