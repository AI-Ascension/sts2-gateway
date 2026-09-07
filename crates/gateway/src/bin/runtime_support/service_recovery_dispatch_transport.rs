// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_gateway::{
    RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST, RUNTIME_V3_SCHEMA_DIGEST, RecoveryHostFence,
    RecoveryOperation,
};

use super::super::recovery_control::{HttpRecoveryControlForwarder, RecoveryControlTransportFault};
use super::super::recovery_frame::{RecoveryFrameError, RecoveryKind, parse_response_frame};
use super::RuntimeService;

impl RuntimeService {
    pub(super) fn forward_recovery_control(
        &self,
        kind: RecoveryKind,
        frame: &[u8],
    ) -> Result<Value, (u16, &'static str)> {
        let (status, value) = self.forward_recovery_control_response(kind, frame)?;
        if !(200..300).contains(&status) {
            return Err((status, "host_recovery_rejected"));
        }
        Ok(value)
    }

    /// For historical reads and reconciliation, a valid host response may be a
    /// protocol-level 404/409. Preserve its closed response frame so the caller
    /// can validate and relay the result without treating it as transport loss.
    pub(super) fn forward_recovery_control_response(
        &self,
        kind: RecoveryKind,
        frame: &[u8],
    ) -> Result<(u16, Value), (u16, &'static str)> {
        let forwarder =
            HttpRecoveryControlForwarder::new(&self.config.mod_address, &self.config.mod_token);
        let response = forwarder
            .forward_frame(kind, frame)
            .map_err(|error| (transport_status(error), transport_reason(error)))?;
        let value = parse_response_frame(&response.body, kind).map_err(|error| match error {
            RecoveryFrameError::Oversized => (502, "host_recovery_response_oversized"),
            RecoveryFrameError::Invalid => (502, "host_recovery_response_invalid"),
        })?;
        if value["correlation_id"] != frame_correlation(frame) {
            return Err((409, "recovery_correlation_mismatch"));
        }
        Ok((response.status, value))
    }

    pub(super) fn recovery_operation_submit_proof(
        &self,
        lease: &sts2_gateway::RecoveryLease,
        operation: &RecoveryOperation,
    ) -> Option<String> {
        let bytes = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            RECOVERY_CONTRACT,
            RECOVERY_SCHEMA_DIGEST,
            RUNTIME_V3_SCHEMA_DIGEST,
            lease.deployment_id,
            lease.instance_id,
            lease.instance_incarnation,
            lease.boot_id,
            lease.authority_generation,
            lease.lease_id,
            lease.lease_epoch,
            lease.fence_token,
            timestamp_for_proof(lease.issued_at_millis),
            timestamp_for_proof(lease.expires_at_millis),
            lease.ttl_seconds,
            lease.renewal_interval_seconds,
            operation.operation_id,
            operation.payload_digest,
        );
        secret_proof("STS2_RUNTIME_BOOTSTRAP_SECRET", "operation-submit", &bytes)
    }

    pub(super) fn recovery_host_fence_proof(
        &self,
        boot: &sts2_gateway::RecoveryBootContext,
    ) -> Option<String> {
        let bytes = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            RECOVERY_CONTRACT,
            RECOVERY_SCHEMA_DIGEST,
            RUNTIME_V3_SCHEMA_DIGEST,
            boot.deployment_id,
            boot.instance_id,
            boot.instance_incarnation,
            boot.boot_id,
            boot.authority_generation,
            boot.release.release_digest,
            boot.release.config_digest,
            boot.release.profile_digest,
            boot.release.runtime_v3_schema_digest,
            timestamp_for_proof(boot.created_at_millis),
            boot_state_name(boot.state),
        );
        secret_proof("STS2_RUNTIME_BOOTSTRAP_SECRET", "host-fence", &bytes)
    }

    pub(super) fn recovery_historical_read_proof(
        &self,
        operation: &RecoveryOperation,
    ) -> Option<String> {
        secret_proof(
            "STS2_RUNTIME_RECOVERY_READ_SECRET",
            "historical-read",
            &operation_control_bytes(operation),
        )
    }

    pub(super) fn recovery_reconcile_proof(
        &self,
        operation: &RecoveryOperation,
        strategy: &str,
        fence: &RecoveryHostFence,
    ) -> Option<String> {
        self.recovery_reconcile_proof_for_context(
            operation.operation_id.as_str(),
            operation.payload_digest.as_str(),
            operation.deployment_id.as_str(),
            operation.instance_id.as_str(),
            operation.instance_incarnation.as_str(),
            operation.boot_id.as_str(),
            operation.authority_generation,
            operation.lease_id.as_str(),
            operation.lease_epoch,
            strategy,
            fence,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn recovery_reconcile_proof_for_context(
        &self,
        operation_id: &str,
        payload_digest: &str,
        deployment_id: &str,
        instance_id: &str,
        instance_incarnation: &str,
        boot_id: &str,
        authority_generation: u64,
        lease_id: &str,
        lease_epoch: u64,
        strategy: &str,
        fence: &RecoveryHostFence,
    ) -> Option<String> {
        let bytes = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            RECOVERY_CONTRACT,
            RECOVERY_SCHEMA_DIGEST,
            RUNTIME_V3_SCHEMA_DIGEST,
            operation_id,
            payload_digest,
            deployment_id,
            instance_id,
            instance_incarnation,
            boot_id,
            authority_generation,
            lease_id,
            lease_epoch,
            strategy,
            fence.host_fence_id,
            fence.deployment_id,
            fence.instance_id,
            fence.instance_incarnation,
            fence.boot_id,
            fence.authority_generation,
            fence.fence_generation,
            timestamp_for_proof(fence.created_at_millis),
        );
        secret_proof(
            "STS2_RUNTIME_RECOVERY_RECONCILE_SECRET",
            "reconcile",
            &bytes,
        )
    }
}

fn operation_control_bytes(operation: &RecoveryOperation) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        RECOVERY_CONTRACT,
        RECOVERY_SCHEMA_DIGEST,
        RUNTIME_V3_SCHEMA_DIGEST,
        operation.operation_id,
        operation.payload_digest,
        operation.deployment_id,
        operation.instance_id,
        operation.instance_incarnation,
        operation.boot_id,
        operation.authority_generation,
        operation.lease_id,
        operation.lease_epoch,
    )
}

fn boot_state_name(state: sts2_gateway::RecoveryBootState) -> &'static str {
    match state {
        sts2_gateway::RecoveryBootState::FenceRequired => "FENCE_REQUIRED",
        sts2_gateway::RecoveryBootState::Ready => "READY",
        sts2_gateway::RecoveryBootState::Blocked => "BLOCKED",
        sts2_gateway::RecoveryBootState::Revoked => "REVOKED",
    }
}

fn timestamp_for_proof(millis: u64) -> String {
    let value = super::super::recovery_frame::timestamp_from_millis(millis);
    value.strip_suffix('Z').unwrap_or(&value).to_owned() + "0000Z"
}

fn secret_proof(variable: &str, purpose: &str, bytes: &str) -> Option<String> {
    use base64::Engine;
    let encoded = std::env::var(variable).ok()?;
    if encoded.is_empty() || encoded.len() > 512 {
        return None;
    }
    let secret = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if secret.len() < 16 {
        return None;
    }
    Some(urlsafe_hmac_sha256(&secret, &format!("{purpose}\n{bytes}")))
}

fn frame_correlation(frame: &[u8]) -> String {
    super::super::strict_json::parse(frame)
        .ok()
        .and_then(|value| value["correlation_id"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn transport_status(error: RecoveryControlTransportFault) -> u16 {
    match error {
        RecoveryControlTransportFault::InvalidFrame => 400,
        RecoveryControlTransportFault::RequestOversized => 413,
        RecoveryControlTransportFault::InvalidConfiguration => 500,
        RecoveryControlTransportFault::UnavailableBeforeWrite => 503,
        RecoveryControlTransportFault::DisconnectedAfterWrite
        | RecoveryControlTransportFault::TimeoutAfterWrite => 503,
        RecoveryControlTransportFault::MalformedResponse => 502,
    }
}

fn transport_reason(error: RecoveryControlTransportFault) -> &'static str {
    match error {
        RecoveryControlTransportFault::InvalidFrame => "host_recovery_request_invalid",
        RecoveryControlTransportFault::RequestOversized => "recovery_frame_oversized",
        RecoveryControlTransportFault::InvalidConfiguration => {
            "recovery_control_configuration_invalid"
        }
        RecoveryControlTransportFault::UnavailableBeforeWrite => "recovery_host_unavailable",
        RecoveryControlTransportFault::DisconnectedAfterWrite
        | RecoveryControlTransportFault::TimeoutAfterWrite => "host_recovery_outcome_unknown",
        RecoveryControlTransportFault::MalformedResponse => "host_recovery_response_invalid",
    }
}

fn urlsafe_hmac_sha256(key: &[u8], message: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let mut normalized = [0_u8; 64];
    if key.len() > normalized.len() {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner = [0_u8; 64];
    let mut outer = [0_u8; 64];
    for index in 0..64 {
        inner[index] = normalized[index] ^ 0x36;
        outer[index] = normalized[index] ^ 0x5c;
    }
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(inner);
    inner_hasher.update(message.as_bytes());
    let inner_digest = inner_hasher.finalize();
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(outer);
    outer_hasher.update(inner_digest);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(outer_hasher.finalize())
}
