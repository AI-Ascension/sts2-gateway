// SPDX-License-Identifier: MIT

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

fn assert_expired(
    kind: HostLeaseKind,
    ack: &HostLeaseAck,
    current_lease: &RecoveryLease,
    renewal: Option<(u64, u64)>,
) {
    assert_eq!(
        validate_ack(
            kind,
            ack,
            current_lease,
            &fence(),
            INSTALLATION_ID,
            &"a".repeat(64),
            renewal,
        ),
        Err(HostLeaseFailure::expired())
    );
}

#[test]
fn delayed_ack_is_rejected_at_the_expiry_boundary() {
    let current_lease = lease(2_000);
    let delayed = install_ack(2_000, current_lease.expires_at_millis);
    assert_expired(HostLeaseKind::Install, &delayed, &current_lease, None);
}

#[test]
fn delayed_renew_ack_is_rejected_at_the_lease_expiry() {
    let current_lease = lease(2_000);
    let mut delayed = install_ack(2_000, 3_000);
    delayed.renew_sequence = Some(1);
    assert_expired(
        HostLeaseKind::Renew,
        &delayed,
        &current_lease,
        Some((1, 3_000)),
    );
}

#[test]
fn delayed_revoke_ack_remains_valid_for_historical_shutdown() {
    let current_lease = lease(2_000);
    let mut delayed = install_ack(2_000, current_lease.expires_at_millis);
    delayed.expires_at = None;
    assert_eq!(
        validate_ack(
            HostLeaseKind::Revoke,
            &delayed,
            &current_lease,
            &fence(),
            INSTALLATION_ID,
            &"a".repeat(64),
            None,
        ),
        Ok(())
    );
}
