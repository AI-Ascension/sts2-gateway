// SPDX-License-Identifier: MIT

//! Wire and proof handling for `watchdog-host-lease-control-v1`.
//!
//! This module intentionally does not reuse the frozen recovery-v1 frame
//! parser. A recovery-v1 `lease_acquire_request` is a gateway-local issuance
//! operation and must never be sent to the host. The host sideband installs
//! the complete gateway-issued grant instead.

use serde_json::{Value, json};
use uuid::Uuid;

use sts2_gateway::{
    HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST, MAX_HOST_LEASE_FRAME_BYTES,
};

use super::host_lease_control_crypto as crypto;
use super::host_lease_control_frames::validate_frame_shape;
use super::recovery_frame::{now_timestamp, timestamp_from_millis};
use super::strict_json;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HostLeaseKind {
    Install,
    Renew,
    Revoke,
}

impl HostLeaseKind {
    pub(super) const fn request_name(self) -> &'static str {
        match self {
            Self::Install => "lease_install_request",
            Self::Renew => "lease_renew_request",
            Self::Revoke => "lease_revoke_request",
        }
    }

    pub(super) const fn response_name(self) -> &'static str {
        match self {
            Self::Install => "lease_install_response",
            Self::Renew => "lease_renew_response",
            Self::Revoke => "lease_revoke_response",
        }
    }

    pub(super) const fn capability(self) -> &'static str {
        match self {
            Self::Install => "lease_install",
            Self::Renew => "lease_renew",
            Self::Revoke => "lease_revoke",
        }
    }

    pub(super) const fn request_domain(self) -> &'static str {
        match self {
            Self::Install => "host-lease-control/v1/lease-install-request",
            Self::Renew => "host-lease-control/v1/lease-renew-request",
            Self::Revoke => "host-lease-control/v1/lease-revoke-request",
        }
    }

    pub(super) const fn acknowledgment_domain(self) -> &'static str {
        match self {
            Self::Install => "host-lease-control/v1/lease-install-ack",
            Self::Renew => "host-lease-control/v1/lease-renew-ack",
            Self::Revoke => "host-lease-control/v1/lease-revoke-ack",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HostLeaseFrameError {
    Invalid,
    Oversized,
    Authentication,
    Configuration,
}

pub(super) fn request_frame(
    kind: HostLeaseKind,
    principal_id: &str,
    correlation_id: &str,
    payload: Value,
    secret: &[u8],
) -> Result<Vec<u8>, HostLeaseFrameError> {
    if !crypto::valid_uuid(principal_id)
        || !crypto::valid_uuid_v4(correlation_id)
        || secret.len() != 32
    {
        return Err(HostLeaseFrameError::Configuration);
    }
    let mut frame = json!({
        "contract": HOST_LEASE_CONTROL_CONTRACT,
        "schema_digest": HOST_LEASE_CONTROL_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation_id,
        "sent_at": now_timestamp(),
        "actor": { "principal_id": principal_id, "role": "gateway" },
        "auth": {
            "principal_id": principal_id,
            "capability": kind.capability(),
            "proof": Value::Null,
        },
        "kind": kind.request_name(),
        "payload": payload,
    });
    let proof = crypto::proof_for_frame(&frame, kind.request_domain(), secret);
    frame["auth"]["proof"] = Value::String(proof);
    validate_frame_shape(&frame, kind, false)?;
    let bytes = serde_json::to_vec(&frame).map_err(|_| HostLeaseFrameError::Invalid)?;
    if bytes.len() > MAX_HOST_LEASE_FRAME_BYTES {
        return Err(HostLeaseFrameError::Oversized);
    }
    Ok(bytes)
}

pub(super) fn parse_request(
    bytes: &[u8],
    kind: HostLeaseKind,
) -> Result<Value, HostLeaseFrameError> {
    parse_frame(bytes, kind, false)
}

pub(super) fn parse_response(
    bytes: &[u8],
    kind: HostLeaseKind,
    secret: &[u8],
    expected_principal_id: &str,
) -> Result<Value, HostLeaseFrameError> {
    if secret.len() != 32 {
        return Err(HostLeaseFrameError::Configuration);
    }
    let value = parse_frame(bytes, kind, true)?;
    if value["actor"]["principal_id"].as_str() != Some(expected_principal_id)
        || value["auth"]["principal_id"].as_str() != Some(expected_principal_id)
    {
        return Err(HostLeaseFrameError::Authentication);
    }
    let Some(proof) = value["auth"]["proof"].as_str() else {
        return Err(HostLeaseFrameError::Authentication);
    };
    if !crypto::constant_time_equal(
        proof.as_bytes(),
        crypto::proof_for_frame(&value, kind.acknowledgment_domain(), secret).as_bytes(),
    ) {
        return Err(HostLeaseFrameError::Authentication);
    }
    Ok(value)
}

pub(super) fn verify_request_proof(
    value: &Value,
    kind: HostLeaseKind,
    secret: &[u8],
) -> Result<(), HostLeaseFrameError> {
    if secret.len() != 32 {
        return Err(HostLeaseFrameError::Configuration);
    }
    validate_frame_shape(value, kind, false)?;
    let Some(proof) = value["auth"]["proof"].as_str() else {
        return Err(HostLeaseFrameError::Authentication);
    };
    if !crypto::constant_time_equal(
        proof.as_bytes(),
        crypto::proof_for_frame(value, kind.request_domain(), secret).as_bytes(),
    ) {
        return Err(HostLeaseFrameError::Authentication);
    }
    Ok(())
}

pub(super) fn grant_digest(grant: &Value) -> Result<String, HostLeaseFrameError> {
    crypto::grant_digest(grant)
}

pub(super) fn canonical_hcj1(value: &Value) -> Result<Vec<u8>, HostLeaseFrameError> {
    crypto::canonical_hcj1(value)
}

pub(super) fn host_lease_key_from_environment() -> Result<Vec<u8>, HostLeaseFrameError> {
    use base64::Engine;
    let encoded = std::env::var("STS2_RUNTIME_HOST_LEASE_KEY")
        .map_err(|_| HostLeaseFrameError::Configuration)?
        .trim()
        .to_owned();
    let secret = if encoded.len() == 64 && encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let mut bytes = Vec::with_capacity(32);
        for pair in encoded.as_bytes().chunks_exact(2) {
            let high = hex_value(pair[0]).ok_or(HostLeaseFrameError::Configuration)?;
            let low = hex_value(pair[1]).ok_or(HostLeaseFrameError::Configuration)?;
            bytes.push((high << 4) | low);
        }
        bytes
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| HostLeaseFrameError::Configuration)?
    };
    if secret.len() != 32 {
        return Err(HostLeaseFrameError::Configuration);
    }
    Ok(secret)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn ack_status(value: &Value, kind: HostLeaseKind) -> Option<&str> {
    let status = value["payload"]["ack"]["result"]["status"].as_str()?;
    let accepted = match kind {
        HostLeaseKind::Install => ["INSTALLED", "DUPLICATE"].as_slice(),
        HostLeaseKind::Renew => ["RENEWED", "RENEW_DUPLICATE"].as_slice(),
        HostLeaseKind::Revoke => ["REVOKED", "REVOKE_DUPLICATE"].as_slice(),
    };
    accepted.contains(&status).then_some(status)
}

pub(super) fn ack_context(value: &Value) -> Option<HostLeaseAck> {
    let ack = value["payload"]["ack"].as_object()?;
    Some(HostLeaseAck {
        installation_id: ack["installation_id"].as_str()?.to_owned(),
        grant_digest: ack["grant_digest"].as_str()?.to_owned(),
        boot_id: ack["boot_id"].as_str()?.to_owned(),
        instance_incarnation: ack["instance_incarnation"].as_str()?.to_owned(),
        host_fence_id: ack["host_fence_id"].as_str()?.to_owned(),
        fence_generation: ack["fence_generation"].as_u64()?,
        lease_id: ack["lease_id"].as_str()?.to_owned(),
        lease_epoch: ack["lease_epoch"].as_u64()?,
        host_install_generation: ack["host_install_generation"].as_u64()?,
        recorded_at: crypto::parse_timestamp_millis(ack["recorded_at"].as_str()?)?,
        renew_sequence: ack["renew_sequence"].as_u64(),
        expires_at: ack["expires_at"]
            .as_str()
            .and_then(crypto::parse_timestamp_millis),
        message_id: value["message_id"].as_str()?.to_owned(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HostLeaseAck {
    pub(super) installation_id: String,
    pub(super) grant_digest: String,
    pub(super) boot_id: String,
    pub(super) instance_incarnation: String,
    pub(super) host_fence_id: String,
    pub(super) fence_generation: u64,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
    pub(super) host_install_generation: u64,
    pub(super) recorded_at: u64,
    pub(super) renew_sequence: Option<u64>,
    pub(super) expires_at: Option<u64>,
    pub(super) message_id: String,
}

fn parse_frame(
    bytes: &[u8],
    kind: HostLeaseKind,
    response: bool,
) -> Result<Value, HostLeaseFrameError> {
    if bytes.is_empty() {
        return Err(HostLeaseFrameError::Invalid);
    }
    if bytes.len() > MAX_HOST_LEASE_FRAME_BYTES {
        return Err(HostLeaseFrameError::Oversized);
    }
    let value = strict_json::parse_hcj1(bytes).map_err(|_| HostLeaseFrameError::Invalid)?;
    validate_frame_shape(&value, kind, response)?;
    Ok(value)
}

#[allow(dead_code)]
fn timestamp_for_grant(millis: u64) -> String {
    timestamp_from_millis(millis)
}

#[allow(dead_code)]
fn _canonical_marker(value: &Value) -> Result<Vec<u8>, HostLeaseFrameError> {
    canonical_hcj1(value)
}

#[cfg(test)]
#[path = "host_lease_control_tests.rs"]
mod tests;
