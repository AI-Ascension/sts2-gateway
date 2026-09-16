// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::RecoveryHostFence;

/// Non-secret identity of the gateway's current owner for one live destination.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryContinuationOwner {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub host_fence_id: String,
    pub host_fence_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub session_id: String,
    pub lease_expires_at_millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryContinuationOwnerState {
    Available,
    Absent,
    Expired,
    Revoked,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryContinuationOwnerSnapshot {
    pub state: RecoveryContinuationOwnerState,
    pub owner: Option<RecoveryContinuationOwner>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryContinuationOwnerClaim {
    pub operation_id: String,
    pub request_digest: String,
    pub owner: RecoveryContinuationOwner,
    pub claimed_at_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryContinuationOwnerClaimResult {
    Created(RecoveryContinuationOwnerClaim),
    Duplicate(RecoveryContinuationOwnerClaim),
}

/// Read-only result for adopting one retained claim onto its still-live owner.
///
/// This contains no lease token or host proof. The fence is the durable row
/// used to reconstruct the public allocation recovery authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryContinuationOwnerAdoption {
    pub claim: RecoveryContinuationOwnerClaim,
    pub owner: RecoveryContinuationOwner,
    pub current_fence: RecoveryHostFence,
}
