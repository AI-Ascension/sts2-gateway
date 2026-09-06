// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

pub use super::recovery_validation::RecoveryStoreError;

#[path = "recovery_types_ticket.rs"]
mod recovery_types_ticket;

pub use recovery_types_ticket::{
    RecoveryAdmissionTicket, RecoveryStoreConfig, RecoveryTicketState,
};

pub const RECOVERY_CONTRACT: &str = "watchdog-recovery-v1";
pub const RECOVERY_SCHEMA_DIGEST: &str =
    "fb934d3157485aaf6e13e6ebbb213ec8a14c7fc6f5eeebc06b7a22c1f0009217";
pub const RUNTIME_V3_SCHEMA_DIGEST: &str =
    "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63";
pub const MAX_RECOVERY_FRAME_BYTES: usize = 262_144;
pub const MAX_RECOVERY_ACTION_BYTES: usize = 65_536;
pub const MAX_RECOVERY_RESPONSE_BYTES: usize = 128 * 1024;
pub const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;
pub const RECOVERY_TOMBSTONE_RETENTION_MILLIS: u64 = 86_400_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryReleaseSet {
    pub release_digest: String,
    pub config_digest: String,
    pub profile_digest: String,
    pub runtime_v3_schema_digest: String,
}

impl RecoveryReleaseSet {
    pub fn new(
        release_digest: &str,
        config_digest: &str,
        profile_digest: &str,
        runtime_v3_schema_digest: &str,
    ) -> Result<Self, RecoveryStoreError> {
        let release = Self {
            release_digest: release_digest.to_owned(),
            config_digest: config_digest.to_owned(),
            profile_digest: profile_digest.to_owned(),
            runtime_v3_schema_digest: runtime_v3_schema_digest.to_owned(),
        };
        release.validate()?;
        Ok(release)
    }

    pub fn unconfigured() -> Self {
        let zero = "0".repeat(64);
        Self {
            release_digest: zero.clone(),
            config_digest: zero.clone(),
            profile_digest: zero,
            runtime_v3_schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
        }
    }

    pub fn validate(&self) -> Result<(), RecoveryStoreError> {
        for (name, value) in [
            ("release_digest", &self.release_digest),
            ("config_digest", &self.config_digest),
            ("profile_digest", &self.profile_digest),
            ("runtime_v3_schema_digest", &self.runtime_v3_schema_digest),
        ] {
            if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(RecoveryStoreError::InvalidInput(format!(
                    "{name} must be a lowercase SHA-256 digest"
                )));
            }
            if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
                return Err(RecoveryStoreError::InvalidInput(format!(
                    "{name} must be lowercase"
                )));
            }
        }
        if self.runtime_v3_schema_digest != RUNTIME_V3_SCHEMA_DIGEST {
            return Err(RecoveryStoreError::ContractMismatch(
                "runtime-v3 schema digest is not the approved frozen profile".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryBootState {
    FenceRequired,
    Ready,
    Blocked,
    Revoked,
}

impl RecoveryBootState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::FenceRequired => "FENCE_REQUIRED",
            Self::Ready => "READY",
            Self::Blocked => "BLOCKED",
            Self::Revoked => "REVOKED",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, RecoveryStoreError> {
        match value {
            "FENCE_REQUIRED" => Ok(Self::FenceRequired),
            "READY" => Ok(Self::Ready),
            "BLOCKED" => Ok(Self::Blocked),
            "REVOKED" => Ok(Self::Revoked),
            _ => Err(RecoveryStoreError::Corrupt("unknown boot state".to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryBootContext {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub release: RecoveryReleaseSet,
    pub created_at_millis: u64,
    pub state: RecoveryBootState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryHostFence {
    pub host_fence_id: String,
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub fence_generation: u64,
    pub created_at_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryLease {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub fence_token: String,
    pub issued_at_millis: u64,
    pub expires_at_millis: u64,
    pub ttl_seconds: u64,
    pub renewal_interval_seconds: u64,
    pub last_renew_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryLeaseProof {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub fence_token: String,
}

impl RecoveryLease {
    pub fn proof(&self) -> RecoveryLeaseProof {
        RecoveryLeaseProof {
            deployment_id: self.deployment_id.clone(),
            instance_id: self.instance_id.clone(),
            instance_incarnation: self.instance_incarnation.clone(),
            boot_id: self.boot_id.clone(),
            authority_generation: self.authority_generation,
            lease_id: self.lease_id.clone(),
            lease_epoch: self.lease_epoch,
            fence_token: self.fence_token.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryLeaseState {
    Active,
    Expired,
    Revoked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryOperationState {
    IntentRecorded,
    MayHaveBeenDispatched,
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Reconciled,
}

impl RecoveryOperationState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IntentRecorded => "INTENT_RECORDED",
            Self::MayHaveBeenDispatched => "MAY_HAVE_BEEN_DISPATCHED",
            Self::Accepted => "ACCEPTED",
            Self::Settled => "SETTLED",
            Self::Rejected => "REJECTED",
            Self::Unknown => "UNKNOWN",
            Self::Reconciled => "RECONCILED",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, RecoveryStoreError> {
        match value {
            "INTENT_RECORDED" => Ok(Self::IntentRecorded),
            "MAY_HAVE_BEEN_DISPATCHED" => Ok(Self::MayHaveBeenDispatched),
            "ACCEPTED" => Ok(Self::Accepted),
            "SETTLED" => Ok(Self::Settled),
            "REJECTED" => Ok(Self::Rejected),
            "UNKNOWN" => Ok(Self::Unknown),
            "RECONCILED" => Ok(Self::Reconciled),
            _ => Err(RecoveryStoreError::Corrupt(
                "unknown operation state".to_owned(),
            )),
        }
    }

    pub(crate) const fn unresolved(self) -> bool {
        matches!(
            self,
            Self::IntentRecorded | Self::MayHaveBeenDispatched | Self::Accepted | Self::Unknown
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryUncertaintyReason {
    TransportLost,
    Timeout,
    GatewayCrash,
    HostCrash,
    ReceiptMissing,
    AuthorityRotated,
}

impl RecoveryUncertaintyReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TransportLost => "transport_lost",
            Self::Timeout => "timeout",
            Self::GatewayCrash => "gateway_crash",
            Self::HostCrash => "host_crash",
            Self::ReceiptMissing => "receipt_missing",
            Self::AuthorityRotated => "authority_rotated",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryEffectWitness {
    pub witness_id: String,
    pub operation_id: String,
    pub payload_digest: String,
    pub boot_id: String,
    pub instance_incarnation: String,
    pub host_fence_id: String,
    pub source: String,
    pub state_id: String,
    pub generation: u64,
    pub effect_digest: String,
    pub observed_at_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryOperation {
    pub operation_id: String,
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub state: RecoveryOperationState,
    pub payload_digest: String,
    pub schema_digest: String,
    pub canonical_json: Vec<u8>,
    pub expected_state_id: String,
    pub expected_generation: u64,
    pub catalog_digest: String,
    pub response_status: Option<u16>,
    pub response_body: Option<Vec<u8>>,
    pub witness: Option<RecoveryEffectWitness>,
    pub uncertainty_reason: Option<RecoveryUncertaintyReason>,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryOperationIntent {
    pub operation_id: String,
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub schema_digest: String,
    pub canonical_json: Vec<u8>,
    pub payload_digest: String,
    pub expected_state_id: String,
    pub expected_generation: u64,
    pub catalog_digest: String,
    pub now_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryIntentResult {
    Created(RecoveryOperation),
    Duplicate(RecoveryOperation),
}

pub(crate) use super::recovery_validation::{
    validate_digest, validate_identity, validate_token, validate_uuid, validate_uuid_v4,
    validate_wire,
};
