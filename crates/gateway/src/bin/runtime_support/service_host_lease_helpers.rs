// SPDX-License-Identifier: MIT

//! Shared construction and error mapping for host lease operations.

use serde_json::{Value, json};
use sts2_gateway::{RecoveryBootContext, RecoveryHostFence, RecoveryLease, RecoveryStoreError};

use super::super::host_lease_control::{HostLeaseAck, HostLeaseFrameError};
use super::super::recovery_frame::timestamp_from_millis;
use super::host_lease::HostLeaseFailure;

pub(super) fn grant_value(
    boot: &RecoveryBootContext,
    fence: &RecoveryHostFence,
    lease: &RecoveryLease,
    principal_id: &str,
    session_id: &str,
) -> Value {
    json!({
        "boot": {
            "deployment_id": boot.deployment_id,
            "instance_id": boot.instance_id,
            "instance_incarnation": boot.instance_incarnation,
            "boot_id": boot.boot_id,
            "authority_generation": boot.authority_generation,
            "release": boot.release,
            "created_at": timestamp_from_millis(boot.created_at_millis),
            "state": "READY",
        },
        "fence": {
            "host_fence_id": fence.host_fence_id,
            "deployment_id": fence.deployment_id,
            "instance_id": fence.instance_id,
            "instance_incarnation": fence.instance_incarnation,
            "boot_id": fence.boot_id,
            "authority_generation": fence.authority_generation,
            "fence_generation": fence.fence_generation,
            "created_at": timestamp_from_millis(fence.created_at_millis),
        },
        "lease": {
            "deployment_id": lease.deployment_id,
            "instance_id": lease.instance_id,
            "instance_incarnation": lease.instance_incarnation,
            "boot_id": lease.boot_id,
            "authority_generation": lease.authority_generation,
            "host_fence_id": fence.host_fence_id,
            "host_fence_generation": fence.fence_generation,
            "lease_id": lease.lease_id,
            "lease_epoch": lease.lease_epoch,
            "fence_token": lease.fence_token,
            "issued_at": timestamp_from_millis(lease.issued_at_millis),
            "expires_at": timestamp_from_millis(lease.expires_at_millis),
            "ttl_seconds": lease.ttl_seconds,
            "renewal_interval_seconds": lease.renewal_interval_seconds,
        },
        "release": boot.release,
        "gateway": {
            "principal_id": principal_id,
            "instance_id": boot.instance_id,
            "session_id": session_id,
        },
    })
}

pub(super) fn validate_ack(
    ack: &HostLeaseAck,
    lease: &RecoveryLease,
    fence: &RecoveryHostFence,
    installation_id: &str,
    grant_digest: &str,
    renewal: Option<(u64, u64)>,
) -> Result<(), HostLeaseFailure> {
    if ack.installation_id != installation_id
        || ack.grant_digest != grant_digest
        || ack.boot_id != lease.boot_id
        || ack.instance_incarnation != lease.instance_incarnation
        || ack.host_fence_id != fence.host_fence_id
        || ack.fence_generation != fence.fence_generation
        || ack.lease_id != lease.lease_id
        || ack.lease_epoch != lease.lease_epoch
    {
        return Err(HostLeaseFailure {
            status: 409,
            code: "recovery_host_lease_context_mismatch",
        });
    }
    if let Some((sequence, expires)) = renewal
        && (ack.renew_sequence != Some(sequence) || ack.expires_at != Some(expires))
    {
        return Err(HostLeaseFailure {
            status: 409,
            code: "recovery_host_lease_context_mismatch",
        });
    }
    if ack.host_install_generation == 0 || ack.message_id.is_empty() {
        return Err(HostLeaseFailure::invalid_response());
    }
    Ok(())
}

pub(super) fn frame_correlation(frame: &[u8]) -> String {
    super::super::strict_json::parse(frame)
        .ok()
        .and_then(|value| value["correlation_id"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(super) fn map_frame_error(error: HostLeaseFrameError) -> HostLeaseFailure {
    match error {
        HostLeaseFrameError::Configuration => HostLeaseFailure::configuration(),
        HostLeaseFrameError::Oversized => HostLeaseFailure {
            status: 413,
            code: "recovery_host_lease_frame_oversized",
        },
        HostLeaseFrameError::Authentication | HostLeaseFrameError::Invalid => {
            HostLeaseFailure::invalid_response()
        }
    }
}

pub(super) fn map_store_error(error: RecoveryStoreError) -> HostLeaseFailure {
    match error {
        RecoveryStoreError::Busy
        | RecoveryStoreError::Io(_)
        | RecoveryStoreError::Sql(_)
        | RecoveryStoreError::Corrupt(_)
        | RecoveryStoreError::PersistenceUnavailable
        | RecoveryStoreError::AuthorityNotFound
        | RecoveryStoreError::CounterExhausted => HostLeaseFailure {
            status: 503,
            code: "recovery_persistence_unavailable",
        },
        RecoveryStoreError::HostFenceRequired | RecoveryStoreError::AuthorityBlocked => {
            HostLeaseFailure {
                status: 503,
                code: "recovery_host_not_ready",
            }
        }
        RecoveryStoreError::LeaseExpired => HostLeaseFailure {
            status: 410,
            code: "recovery_lease_expired",
        },
        RecoveryStoreError::LeaseRevoked
        | RecoveryStoreError::StaleLease
        | RecoveryStoreError::OperationConflict
        | RecoveryStoreError::InvalidTransition
        | RecoveryStoreError::ContractMismatch(_)
        | RecoveryStoreError::ReleaseMismatch => HostLeaseFailure {
            status: 409,
            code: "recovery_host_lease_conflict",
        },
        RecoveryStoreError::LeaseNotFound => HostLeaseFailure {
            status: 404,
            code: "recovery_lease_not_found",
        },
        RecoveryStoreError::InvalidInput(_) => HostLeaseFailure {
            status: 400,
            code: "recovery_host_lease_invalid",
        },
        RecoveryStoreError::IncompatibleSchema { .. } | RecoveryStoreError::BackupExists => {
            HostLeaseFailure {
                status: 503,
                code: "recovery_persistence_unavailable",
            }
        }
        RecoveryStoreError::OperationNotFound
        | RecoveryStoreError::AdmissionTicketExpired
        | RecoveryStoreError::AdmissionTicketNotFound
        | RecoveryStoreError::CapacityExceeded => HostLeaseFailure {
            status: 409,
            code: "recovery_host_lease_conflict",
        },
    }
}
