// SPDX-License-Identifier: MIT

//! Shared construction and error mapping for host lease operations.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{RecoveryBootContext, RecoveryHostFence, RecoveryLease, RecoveryStoreError};

use super::super::host_lease_control::{HostLeaseAck, HostLeaseFrameError, HostLeaseKind};
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
    kind: HostLeaseKind,
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
    let (expected_sequence, expected_expires) = match kind {
        HostLeaseKind::Install => (None, Some(lease.expires_at_millis)),
        HostLeaseKind::Renew => renewal
            .map(|(sequence, expires)| (Some(sequence), Some(expires)))
            .ok_or_else(HostLeaseFailure::invalid_response)?,
        HostLeaseKind::Revoke => (None, None),
    };
    if ack.renew_sequence != expected_sequence || ack.expires_at != expected_expires {
        return Err(HostLeaseFailure {
            status: 409,
            code: "recovery_host_lease_context_mismatch",
        });
    }
    if ack.recorded_at >= lease.expires_at_millis {
        return Err(HostLeaseFailure::expired());
    }
    if ack.host_install_generation == 0 || ack.message_id.is_empty() {
        return Err(HostLeaseFailure::invalid_response());
    }
    Ok(())
}

pub(super) fn lease_deadline(
    expires_at_millis: u64,
    ttl_seconds: u64,
    received_at: Instant,
    received_at_wall_millis: u64,
) -> Result<Instant, HostLeaseFailure> {
    let remaining = expires_at_millis
        .checked_sub(received_at_wall_millis)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(HostLeaseFailure::expired)?;
    let ttl_millis = ttl_seconds
        .checked_mul(1_000)
        .ok_or_else(HostLeaseFailure::configuration)?;
    let remaining = remaining.min(ttl_millis);
    received_at
        .checked_add(Duration::from_millis(remaining))
        .ok_or_else(HostLeaseFailure::configuration)
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

#[cfg(test)]
mod tests {
    use sts2_gateway::{RecoveryHostFence, RecoveryLease};

    use super::super::super::host_lease_control::{HostLeaseAck, HostLeaseKind};
    use super::{HostLeaseFailure, validate_ack};

    const BOOT_ID: &str = "00000000-0000-4000-8000-000000000004";
    const INSTANCE_INCAR: &str = "00000000-0000-4000-8000-000000000003";
    const LEASE_ID: &str = "00000000-0000-4000-8000-000000000006";
    const FENCE_ID: &str = "00000000-0000-4000-8000-000000000005";
    const INSTALLATION_ID: &str = "00000000-0000-4000-8000-000000000009";
    const ACK_ID: &str = "00000000-0000-4000-8000-000000000013";

    fn lease(expires_at_millis: u64) -> RecoveryLease {
        RecoveryLease {
            deployment_id: String::from("00000000-0000-4000-8000-000000000001"),
            instance_id: String::from("00000000-0000-4000-8000-000000000002"),
            instance_incarnation: INSTANCE_INCAR.to_owned(),
            boot_id: BOOT_ID.to_owned(),
            authority_generation: 1,
            lease_id: LEASE_ID.to_owned(),
            lease_epoch: 1,
            fence_token: String::from("A").repeat(43),
            issued_at_millis: expires_at_millis.saturating_sub(30_000),
            expires_at_millis,
            ttl_seconds: 30,
            renewal_interval_seconds: 10,
            last_renew_sequence: 0,
        }
    }

    fn fence() -> RecoveryHostFence {
        RecoveryHostFence {
            host_fence_id: FENCE_ID.to_owned(),
            deployment_id: String::from("00000000-0000-4000-8000-000000000001"),
            instance_id: String::from("00000000-0000-4000-8000-000000000002"),
            instance_incarnation: INSTANCE_INCAR.to_owned(),
            boot_id: BOOT_ID.to_owned(),
            authority_generation: 1,
            fence_generation: 3,
            created_at_millis: 1,
        }
    }

    fn install_ack(recorded_at: u64, expires_at: u64) -> HostLeaseAck {
        HostLeaseAck {
            installation_id: INSTALLATION_ID.to_owned(),
            grant_digest: "a".repeat(64),
            boot_id: BOOT_ID.to_owned(),
            instance_incarnation: INSTANCE_INCAR.to_owned(),
            host_fence_id: FENCE_ID.to_owned(),
            fence_generation: 3,
            lease_id: LEASE_ID.to_owned(),
            lease_epoch: 1,
            host_install_generation: 1,
            recorded_at,
            renew_sequence: None,
            expires_at: Some(expires_at),
            message_id: ACK_ID.to_owned(),
        }
    }

    #[test]
    fn delayed_ack_is_rejected_at_the_expiry_boundary() {
        let current_lease = lease(2_000);
        let delayed = install_ack(2_000, current_lease.expires_at_millis);
        assert_eq!(
            validate_ack(
                HostLeaseKind::Install,
                &delayed,
                &current_lease,
                &fence(),
                INSTALLATION_ID,
                &"a".repeat(64),
                None,
            ),
            Err(HostLeaseFailure::expired())
        );
    }
}
