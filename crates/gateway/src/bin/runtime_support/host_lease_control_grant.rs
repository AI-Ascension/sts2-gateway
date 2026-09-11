// SPDX-License-Identifier: MIT

//! Validation of the closed gateway-issued host lease grant.

use serde_json::{Map, Value};
use sts2_gateway::MAX_WIRE_INTEGER;

use super::host_lease_control::{HostLeaseFrameError, HostLeaseKind};
use super::host_lease_control_crypto::{
    parse_timestamp_millis, valid_digest, valid_timestamp, valid_token, valid_uuid, valid_uuid_v4,
};

pub(super) fn validate_grant(value: &Value) -> Result<(), HostLeaseFrameError> {
    let grant = value.as_object().ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(grant, &["boot", "fence", "lease", "release", "gateway"])?;
    let boot = grant["boot"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        boot,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "release",
            "created_at",
            "state",
        ],
    )?;
    if !boot["deployment_id"].as_str().is_some_and(valid_uuid)
        || !boot["instance_id"].as_str().is_some_and(valid_uuid)
        || !boot["instance_incarnation"]
            .as_str()
            .is_some_and(valid_uuid_v4)
        || !boot["boot_id"].as_str().is_some_and(valid_uuid_v4)
        || !boot["authority_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || boot["state"].as_str() != Some("READY")
        || !valid_timestamp(boot["created_at"].as_str().unwrap_or_default())
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    validate_release(&boot["release"])?;
    let fence = grant["fence"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        fence,
        &[
            "host_fence_id",
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "fence_generation",
            "created_at",
        ],
    )?;
    if !fence["host_fence_id"].as_str().is_some_and(valid_uuid_v4)
        || !fence["deployment_id"].as_str().is_some_and(valid_uuid)
        || !fence["instance_id"].as_str().is_some_and(valid_uuid)
        || !fence["instance_incarnation"]
            .as_str()
            .is_some_and(valid_uuid_v4)
        || !fence["boot_id"].as_str().is_some_and(valid_uuid_v4)
        || !fence["authority_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !fence["fence_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !valid_timestamp(fence["created_at"].as_str().unwrap_or_default())
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    let lease = grant["lease"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        lease,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "host_fence_id",
            "host_fence_generation",
            "lease_id",
            "lease_epoch",
            "fence_token",
            "issued_at",
            "expires_at",
            "ttl_seconds",
            "renewal_interval_seconds",
        ],
    )?;
    if !lease["deployment_id"].as_str().is_some_and(valid_uuid)
        || !lease["instance_id"].as_str().is_some_and(valid_uuid)
        || !lease["instance_incarnation"]
            .as_str()
            .is_some_and(valid_uuid_v4)
        || !lease["boot_id"].as_str().is_some_and(valid_uuid_v4)
        || !lease["authority_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !lease["host_fence_id"].as_str().is_some_and(valid_uuid_v4)
        || !lease["host_fence_generation"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !lease["lease_id"].as_str().is_some_and(valid_uuid_v4)
        || !lease["lease_epoch"]
            .as_u64()
            .is_some_and(|value| (1..=MAX_WIRE_INTEGER).contains(&value))
        || !lease["fence_token"].as_str().is_some_and(valid_token)
        || !valid_timestamp(lease["issued_at"].as_str().unwrap_or_default())
        || !valid_timestamp(lease["expires_at"].as_str().unwrap_or_default())
        || !lease["ttl_seconds"]
            .as_u64()
            .is_some_and(|value| (1..=300).contains(&value))
        || !lease["renewal_interval_seconds"]
            .as_u64()
            .is_some_and(|value| {
                (1..lease["ttl_seconds"].as_u64().unwrap_or_default()).contains(&value)
            })
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    let issued_at = parse_timestamp_millis(lease["issued_at"].as_str().unwrap_or_default())
        .ok_or(HostLeaseFrameError::Invalid)?;
    let expires_at = parse_timestamp_millis(lease["expires_at"].as_str().unwrap_or_default())
        .ok_or(HostLeaseFrameError::Invalid)?;
    if expires_at <= issued_at {
        return Err(HostLeaseFrameError::Invalid);
    }
    let release = grant["release"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    validate_release(&Value::Object(release.clone()))?;
    let gateway = grant["gateway"]
        .as_object()
        .ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(gateway, &["principal_id", "instance_id", "session_id"])?;
    if !gateway["principal_id"].as_str().is_some_and(valid_uuid)
        || !gateway["instance_id"].as_str().is_some_and(valid_uuid)
        || !gateway["session_id"].as_str().is_some_and(valid_uuid_v4)
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    for field in [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
    ] {
        if boot[field] != fence[field] || boot[field] != lease[field] {
            return Err(HostLeaseFrameError::Invalid);
        }
    }
    if fence["host_fence_id"] != lease["host_fence_id"]
        || fence["fence_generation"] != lease["host_fence_generation"]
        || boot["release"] != grant["release"]
        || gateway["instance_id"] != boot["instance_id"]
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    Ok(())
}

fn validate_release(value: &Value) -> Result<(), HostLeaseFrameError> {
    let release = value.as_object().ok_or(HostLeaseFrameError::Invalid)?;
    exact_keys(
        release,
        &[
            "release_digest",
            "config_digest",
            "profile_digest",
            "runtime_v3_schema_digest",
        ],
    )?;
    if [
        "release_digest",
        "config_digest",
        "profile_digest",
        "runtime_v3_schema_digest",
    ]
    .iter()
    .any(|field| !release[*field].as_str().is_some_and(valid_digest))
    {
        return Err(HostLeaseFrameError::Invalid);
    }
    Ok(())
}

pub(super) fn exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), HostLeaseFrameError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(HostLeaseFrameError::Invalid);
    }
    Ok(())
}

#[allow(dead_code)]
fn _kind_marker(_kind: HostLeaseKind) {}
