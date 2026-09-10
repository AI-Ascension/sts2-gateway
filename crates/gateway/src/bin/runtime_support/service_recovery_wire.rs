// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::{
    RecoveryBootContext, RecoveryBootState, RecoveryHostFence, RecoveryLease, RecoveryReleaseSet,
    RecoveryStoreError,
};
use uuid::{Uuid, Variant};

use super::super::recovery_control::RecoveryControlTransportFault;
use super::{json_bytes, json_error};

pub(super) fn recovery_control_error(error: RecoveryControlTransportFault) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RecoveryControlTransportFault::InvalidFrame => (400, "recovery_host_fence_frame_invalid"),
        RecoveryControlTransportFault::RequestOversized => (413, "recovery_frame_oversized"),
        RecoveryControlTransportFault::InvalidConfiguration => {
            (500, "recovery_control_configuration_invalid")
        }
        RecoveryControlTransportFault::UnavailableBeforeWrite => (503, "recovery_host_unavailable"),
        RecoveryControlTransportFault::DisconnectedAfterWrite
        | RecoveryControlTransportFault::TimeoutAfterWrite => {
            (503, "recovery_host_fence_outcome_unknown")
        }
        RecoveryControlTransportFault::MalformedResponse => {
            (503, "recovery_host_fence_outcome_unknown")
        }
    };
    (status, json_error(code))
}

pub(super) fn recovery_store_error(error: RecoveryStoreError) -> (u16, Vec<u8>) {
    let (status, code, retryable) = match error {
        RecoveryStoreError::Busy => (423, "busy", true),
        RecoveryStoreError::Io(_)
        | RecoveryStoreError::Sql(_)
        | RecoveryStoreError::Corrupt(_)
        | RecoveryStoreError::PersistenceUnavailable => (503, "persistence_unavailable", true),
        RecoveryStoreError::AuthorityBlocked | RecoveryStoreError::HostFenceRequired => {
            (503, "host_not_ready", true)
        }
        RecoveryStoreError::LeaseExpired | RecoveryStoreError::AdmissionTicketExpired => {
            (410, "lease_expired", false)
        }
        RecoveryStoreError::LeaseNotFound
        | RecoveryStoreError::OperationNotFound
        | RecoveryStoreError::AdmissionTicketNotFound => (404, "not_found", false),
        RecoveryStoreError::StaleLease => (409, "stale_lease", false),
        RecoveryStoreError::OperationConflict
        | RecoveryStoreError::InvalidTransition
        | RecoveryStoreError::ReleaseMismatch
        | RecoveryStoreError::ContractMismatch(_) => (409, "conflict", false),
        RecoveryStoreError::CapacityExceeded => (413, "bounds_exceeded", true),
        RecoveryStoreError::InvalidInput(_) => (400, "invalid", false),
        RecoveryStoreError::LeaseRevoked => (409, "lease_revoked", false),
        RecoveryStoreError::AuthorityNotFound
        | RecoveryStoreError::CounterExhausted
        | RecoveryStoreError::IncompatibleSchema { .. }
        | RecoveryStoreError::BackupExists => (503, "persistence_unavailable", true),
    };
    (
        status,
        json_bytes(&json!({
            "error_code": format!("recovery_{code}"),
            "retryable": retryable,
            "retry_after_seconds": retryable.then_some(1),
        })),
    )
}

pub(super) fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value && id.get_variant() == Variant::RFC4122
    })
}

pub(super) fn valid_uuid_v4(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value
            && id.get_variant() == Variant::RFC4122
            && id.get_version_num() == 4
    })
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Option<&'a serde_json::Map<String, Value>> {
    let object = value.as_object()?;
    (object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key)))
        .then_some(object)
}

pub(super) fn parse_release(value: &Value) -> Option<RecoveryReleaseSet> {
    let object = exact_object(
        value,
        &[
            "release_digest",
            "config_digest",
            "profile_digest",
            "runtime_v3_schema_digest",
        ],
    )?;
    RecoveryReleaseSet::new(
        object["release_digest"].as_str()?,
        object["config_digest"].as_str()?,
        object["profile_digest"].as_str()?,
        object["runtime_v3_schema_digest"].as_str()?,
    )
    .ok()
}

pub(super) fn boot_wire_shape(value: &Value) -> bool {
    let Some(object) = exact_object(
        value,
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
    ) else {
        return false;
    };
    valid_uuid(object["deployment_id"].as_str().unwrap_or_default())
        && valid_uuid(object["instance_id"].as_str().unwrap_or_default())
        && valid_uuid_v4(object["instance_incarnation"].as_str().unwrap_or_default())
        && valid_uuid_v4(object["boot_id"].as_str().unwrap_or_default())
        && object["authority_generation"]
            .as_u64()
            .is_some_and(|value| value > 0)
        && object["created_at"].as_str().is_some()
        && matches!(
            object["state"].as_str(),
            Some("FENCE_REQUIRED" | "READY" | "BLOCKED" | "REVOKED")
        )
        && parse_release(&object["release"]).is_some()
}

pub(super) fn boot_value(boot: &RecoveryBootContext) -> Value {
    json!({
        "deployment_id": boot.deployment_id,
        "instance_id": boot.instance_id,
        "instance_incarnation": boot.instance_incarnation,
        "boot_id": boot.boot_id,
        "authority_generation": boot.authority_generation,
        "release": boot.release,
        "created_at": super::super::recovery_frame::timestamp_from_millis(boot.created_at_millis),
        "state": boot_state_name(boot.state),
    })
}

pub(super) fn boot_state_name(state: RecoveryBootState) -> &'static str {
    match state {
        RecoveryBootState::FenceRequired => "FENCE_REQUIRED",
        RecoveryBootState::Ready => "READY",
        RecoveryBootState::Blocked => "BLOCKED",
        RecoveryBootState::Revoked => "REVOKED",
    }
}

pub(super) fn parse_fence_wire(value: &Value) -> Option<RecoveryHostFence> {
    let object = exact_object(
        value,
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
    let fence = RecoveryHostFence {
        host_fence_id: object["host_fence_id"].as_str()?.to_owned(),
        deployment_id: object["deployment_id"].as_str()?.to_owned(),
        instance_id: object["instance_id"].as_str()?.to_owned(),
        instance_incarnation: object["instance_incarnation"].as_str()?.to_owned(),
        boot_id: object["boot_id"].as_str()?.to_owned(),
        authority_generation: object["authority_generation"].as_u64()?,
        fence_generation: object["fence_generation"].as_u64()?,
        created_at_millis: 0,
    };
    (valid_uuid_v4(&fence.host_fence_id)
        && valid_uuid(&fence.deployment_id)
        && valid_uuid(&fence.instance_id)
        && valid_uuid_v4(&fence.instance_incarnation)
        && valid_uuid_v4(&fence.boot_id)
        && fence.authority_generation > 0
        && fence.fence_generation > 0
        && object["created_at"].as_str().is_some())
    .then_some(fence)
}

pub(super) fn same_boot_fence(fence: &RecoveryHostFence, boot: &RecoveryBootContext) -> bool {
    fence.deployment_id == boot.deployment_id
        && fence.instance_id == boot.instance_id
        && fence.instance_incarnation == boot.instance_incarnation
        && fence.boot_id == boot.boot_id
        && fence.authority_generation == boot.authority_generation
}

pub(super) fn fence_value(fence: &RecoveryHostFence) -> Value {
    json!({
        "host_fence_id": fence.host_fence_id,
        "deployment_id": fence.deployment_id,
        "instance_id": fence.instance_id,
        "instance_incarnation": fence.instance_incarnation,
        "boot_id": fence.boot_id,
        "authority_generation": fence.authority_generation,
        "fence_generation": fence.fence_generation,
        "created_at": super::super::recovery_frame::timestamp_from_millis(fence.created_at_millis),
    })
}

pub(super) fn parse_lease_wire(value: &Value) -> Option<RecoveryLease> {
    let object = exact_object(
        value,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "lease_id",
            "lease_epoch",
            "fence_token",
            "issued_at",
            "expires_at",
            "ttl_seconds",
            "renewal_interval_seconds",
        ],
    )?;
    let lease = RecoveryLease {
        deployment_id: object["deployment_id"].as_str()?.to_owned(),
        instance_id: object["instance_id"].as_str()?.to_owned(),
        instance_incarnation: object["instance_incarnation"].as_str()?.to_owned(),
        boot_id: object["boot_id"].as_str()?.to_owned(),
        authority_generation: object["authority_generation"].as_u64()?,
        lease_id: object["lease_id"].as_str()?.to_owned(),
        lease_epoch: object["lease_epoch"].as_u64()?,
        fence_token: object["fence_token"].as_str()?.to_owned(),
        issued_at_millis: 0,
        expires_at_millis: 0,
        ttl_seconds: object["ttl_seconds"].as_u64()?,
        renewal_interval_seconds: object["renewal_interval_seconds"].as_u64()?,
        last_renew_sequence: 0,
    };
    (valid_uuid(&lease.deployment_id)
        && valid_uuid(&lease.instance_id)
        && valid_uuid_v4(&lease.instance_incarnation)
        && valid_uuid_v4(&lease.boot_id)
        && valid_uuid_v4(&lease.lease_id)
        && lease.authority_generation > 0
        && lease.lease_epoch > 0
        && lease.fence_token.len() == 43
        && lease.ttl_seconds >= 5
        && lease.renewal_interval_seconds > 0
        && lease.renewal_interval_seconds < lease.ttl_seconds
        && object["issued_at"].as_str().is_some()
        && object["expires_at"].as_str().is_some())
    .then_some(lease)
}

pub(super) fn lease_value(lease: &RecoveryLease) -> Value {
    json!({
        "deployment_id": lease.deployment_id,
        "instance_id": lease.instance_id,
        "instance_incarnation": lease.instance_incarnation,
        "boot_id": lease.boot_id,
        "authority_generation": lease.authority_generation,
        "lease_id": lease.lease_id,
        "lease_epoch": lease.lease_epoch,
        "fence_token": lease.fence_token,
        "issued_at": super::super::recovery_frame::timestamp_from_millis(lease.issued_at_millis),
        "expires_at": super::super::recovery_frame::timestamp_from_millis(lease.expires_at_millis),
        "ttl_seconds": lease.ttl_seconds,
        "renewal_interval_seconds": lease.renewal_interval_seconds,
    })
}
