// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use sts2_gateway::RecoveryLease;

use super::super::test_support::test_service;

fn lease() -> RecoveryLease {
    RecoveryLease {
        deployment_id: String::from("00000000-0000-4000-8000-000000000001"),
        instance_id: String::from("00000000-0000-4000-8000-000000000002"),
        instance_incarnation: String::from("00000000-0000-4000-8000-000000000003"),
        boot_id: String::from("00000000-0000-4000-8000-000000000004"),
        authority_generation: 1,
        lease_id: String::from("00000000-0000-4000-8000-000000000006"),
        lease_epoch: 1,
        fence_token: String::from("A").repeat(43),
        issued_at_millis: 1_000,
        expires_at_millis: 10_000,
        ttl_seconds: 30,
        renewal_interval_seconds: 10,
        last_renew_sequence: 0,
    }
}

#[test]
fn duplicate_install_does_not_refresh_deadline_after_wall_clock_regression() {
    let mut service = test_service().expect("test service");
    let current_lease = lease();
    let original_deadline = Instant::now() + Duration::from_secs(2);
    service.recovery_lease = Some(current_lease.clone());
    service.recovery_lease_deadline = Some(original_deadline);

    service
        .establish_install_deadline(&current_lease, Instant::now(), 9_000)
        .expect("existing deadline is still live");
    service
        .establish_install_deadline(&current_lease, Instant::now(), 8_000)
        .expect("wall-clock regression does not expire the lease");

    assert_eq!(service.recovery_lease_deadline, Some(original_deadline));
}

#[test]
fn recovery_clock_never_moves_backwards() {
    let mut service = test_service().expect("test service");
    service.recovery_clock_wall_millis = 10_000;
    service.recovery_last_now_millis = 20_000;

    assert!(service.recovery_now_millis() >= 20_000);
}
