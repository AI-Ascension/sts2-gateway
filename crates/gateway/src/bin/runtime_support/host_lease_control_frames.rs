// SPDX-License-Identifier: MIT

//! Closed-shape validation for host lease-control requests and acknowledgments.

use serde_json::{Map, Value};
use sts2_gateway::{
    HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST, MAX_HOST_LEASE_PAYLOAD_BYTES,
    MAX_HOST_LEASE_PROOF_BYTES, MAX_WIRE_INTEGER,
};

use super::host_lease_control::{HostLeaseFrameError, HostLeaseKind};
use super::host_lease_control_crypto::{
    grant_digest, valid_digest, valid_timestamp, valid_uuid, valid_uuid_v4,
};
use super::host_lease_control_grant::{exact_keys, validate_grant};

pub(super) fn validate_frame_shape(
    value: &Value,
    kind: HostLeaseKind,
    response: bool,
) -> Result<(), HostLeaseFrameError> {
    let root = value.as_object().ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        root,
        &[
            "contract",
            "schema_digest",
            "message_id",
            "correlation_id",
            "sent_at",
            "actor",
            "auth",
            "kind",
            "payload",
        ],
    )?;
    if value["contract"].as_str() != Some(HOST_LEASE_CONTROL_CONTRACT)
        || value["schema_digest"].as_str() != Some(HOST_LEASE_CONTROL_SCHEMA_DIGEST)
        || value["kind"].as_str()
            != Some(if response {
                kind.response_name()
            } else {
                kind.request_name()
            })
        || !value["message_id"].as_str().is_some_and(valid_uuid_v4)
        || !value["correlation_id"].as_str().is_some_and(valid_uuid_v4)
        || !valid_timestamp(value["sent_at"].as_str().unwrap_or_default())
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    let actor = value["actor"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(actor, &["principal_id", "role"])?;
    if !actor["principal_id"].as_str().is_some_and(valid_uuid)
        || actor["role"].as_str() != Some(if response { "host" } else { "gateway" })
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    let auth = value["auth"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(auth, &["principal_id", "capability", "proof"])?;
    if auth["principal_id"] != actor["principal_id"]
        || auth["principal_id"]
            .as_str()
            .is_none_or(|principal| !valid_uuid(principal))
        || auth["capability"].as_str() != Some(kind.capability())
        || auth["proof"]
            .as_str()
            .is_none_or(|proof| proof.is_empty() || proof.len() > MAX_HOST_LEASE_PROOF_BYTES)
    {
        return Err(if auth["proof"].is_null() && !response {
            HostLeaseFrameError::Invalid
        } else {
            HostLeaseFrameError::Authentication
        });
    }
    let payload = value["payload"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    let payload_bytes = serde_json::to_vec(&value["payload"])
        .map_err(|_| HostLeaseFrameError::Invalid)?
        .len();
    if payload_bytes > MAX_HOST_LEASE_PAYLOAD_BYTES {
        return Err(HostLeaseFrameError::Oversized);
    }
    if response {
        validate_response_payload(payload, kind)
    } else {
        validate_request_payload(payload, kind)?;
        if payload["grant"]["gateway"]["principal_id"] != value["actor"]["principal_id"]
            || payload["grant"]["gateway"]["instance_id"] != payload["grant"]["boot"]["instance_id"]
        {
            return Err(HostLeaseFrameError::Invalid);
        }
        Ok(())
    }
}

fn validate_request_payload(
    payload: &Map<String, Value>,
    kind: HostLeaseKind,
) -> Result<(), HostLeaseFrameError> {
    let expected: &[&str] = match kind {
        HostLeaseKind::Install => &["installation_id", "grant", "grant_digest"],
        HostLeaseKind::Renew => &["installation_id", "grant", "grant_digest", "renew_sequence"],
        HostLeaseKind::Revoke => &["installation_id", "grant", "grant_digest", "reason"],
    };
    exact_keys(payload, expected)?;
    if !payload["installation_id"]
        .as_str()
        .is_some_and(valid_uuid_v4)
        || !payload["grant_digest"].as_str().is_some_and(valid_digest)
        || grant_digest(&payload["grant"])? != payload["grant_digest"].as_str().unwrap_or_default()
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    validate_grant(&payload["grant"])?;
    match kind {
        HostLeaseKind::Renew => {
            if payload["renew_sequence"]
                .as_u64()
                .is_none_or(|sequence| sequence == 0 || sequence > MAX_WIRE_INTEGER)
            {
                return Err(HostLeaseFrameError::Invalid);
            }
        }
        HostLeaseKind::Revoke => {
            if !matches!(
                payload["reason"].as_str(),
                Some(
                    "operator"
                        | "shutdown"
                        | "incarnation_replaced"
                        | "suspend_ambiguous"
                        | "rekey"
                )
            ) {
                return Err(HostLeaseFrameError::Invalid);
            }
        }
        HostLeaseKind::Install => {}
    }
    Ok(())
}

fn validate_response_payload(
    payload: &Map<String, Value>,
    kind: HostLeaseKind,
) -> Result<(), HostLeaseFrameError> {
    exact_keys(payload, &["ack"])?;
    let ack = payload["ack"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        ack,
        &[
            "result",
            "installation_id",
            "grant_digest",
            "boot_id",
            "instance_incarnation",
            "host_fence_id",
            "fence_generation",
            "lease_id",
            "lease_epoch",
            "host_install_generation",
            "recorded_at",
            "renew_sequence",
            "expires_at",
        ],
    )?;
    let result = ack["result"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(result, &["status", "retryable", "retry_after_seconds"])?;
    let statuses: &[&str] = match kind {
        HostLeaseKind::Install => &[
            "INSTALLED",
            "DUPLICATE",
            "CONFLICT",
            "STALE_BOOT",
            "STALE_FENCE",
            "RELEASE_MISMATCH",
            "LEASE_MISMATCH",
            "EXPIRED",
            "AUTH_REQUIRED",
            "PERSISTENCE_UNAVAILABLE",
            "INVALID",
            "BOUNDS_EXCEEDED",
        ],
        HostLeaseKind::Renew => &[
            "RENEWED",
            "RENEW_DUPLICATE",
            "CONFLICT",
            "STALE_BOOT",
            "STALE_FENCE",
            "RELEASE_MISMATCH",
            "LEASE_MISMATCH",
            "EXPIRED",
            "AUTH_REQUIRED",
            "PERSISTENCE_UNAVAILABLE",
            "INVALID",
            "BOUNDS_EXCEEDED",
        ],
        HostLeaseKind::Revoke => &[
            "REVOKED",
            "REVOKE_DUPLICATE",
            "CONFLICT",
            "STALE_BOOT",
            "STALE_FENCE",
            "RELEASE_MISMATCH",
            "LEASE_MISMATCH",
            "EXPIRED",
            "AUTH_REQUIRED",
            "PERSISTENCE_UNAVAILABLE",
            "INVALID",
            "BOUNDS_EXCEEDED",
        ],
    };
    if !result["status"]
        .as_str()
        .is_some_and(|status| statuses.contains(&status))
        || !result["retryable"].is_boolean()
        || !(result["retry_after_seconds"].is_null()
            || result["retry_after_seconds"]
                .as_u64()
                .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value)))
        || !ack["installation_id"].as_str().is_some_and(valid_uuid_v4)
        || !ack["grant_digest"].as_str().is_some_and(valid_digest)
        || !ack["boot_id"].as_str().is_some_and(valid_uuid_v4)
        || !ack["instance_incarnation"]
            .as_str()
            .is_some_and(valid_uuid_v4)
        || !ack["host_fence_id"].as_str().is_some_and(valid_uuid_v4)
        || !ack["fence_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !ack["lease_id"].as_str().is_some_and(valid_uuid_v4)
        || !ack["lease_epoch"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !ack["host_install_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !valid_timestamp(ack["recorded_at"].as_str().unwrap_or_default())
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    if let Some(sequence) = ack["renew_sequence"].as_u64()
        && (sequence == 0 || sequence > MAX_WIRE_INTEGER)
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    if !ack["renew_sequence"].is_null() && ack["expires_at"].as_str().is_none() {
        return Err(HostLeaseFrameError::Invalid);
    }
    if !ack["expires_at"].is_null()
        && !valid_timestamp(ack["expires_at"].as_str().unwrap_or_default())
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    Ok(())
}
