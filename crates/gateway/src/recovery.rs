// SPDX-License-Identifier: MIT

#[path = "recovery_canonical.rs"]
mod recovery_canonical;
#[path = "recovery_store.rs"]
mod recovery_store;
#[path = "recovery_types.rs"]
mod recovery_types;
#[path = "recovery_validation.rs"]
mod recovery_validation;

use base64::Engine;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub use recovery_canonical::canonicalize_recovery_action;
pub use recovery_store::{GatewayRecoveryStore, RecoveryLeaseRequest, RecoveryStorePragmas};
pub use recovery_types::{
    MAX_RECOVERY_ACTION_BYTES, MAX_RECOVERY_FRAME_BYTES, MAX_RECOVERY_RESPONSE_BYTES,
    MAX_WIRE_INTEGER, RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST,
    RECOVERY_TOMBSTONE_RETENTION_MILLIS, RUNTIME_V3_SCHEMA_DIGEST, RecoveryAdmissionTicket,
    RecoveryBootContext, RecoveryBootState, RecoveryEffectWitness, RecoveryHostFence,
    RecoveryIntentResult, RecoveryLease, RecoveryLeaseProof, RecoveryLeaseState, RecoveryOperation,
    RecoveryOperationIntent, RecoveryOperationState, RecoveryReleaseSet, RecoveryStoreConfig,
    RecoveryStoreError, RecoveryTicketState, RecoveryUncertaintyReason,
};

pub(crate) fn random_uuid() -> String {
    Uuid::new_v4().to_string()
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn canonical_json_digest(bytes: &[u8]) -> Result<String, RecoveryStoreError> {
    Ok(sha256_hex(&canonicalize_recovery_action(bytes)?))
}
