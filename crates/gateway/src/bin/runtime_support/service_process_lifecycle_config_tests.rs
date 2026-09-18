// SPDX-License-Identifier: MIT

//! Configuration and identity-bridge tests for the process-lifecycle route surface.
//!
//! Split from `service_process_lifecycle_tests.rs` so the HTTP boundary tests and the pure
//! configuration/bridge tests stay separately readable. These drive the same real composition
//! path; none of them launches a native process.

use super::*;

#[test]
fn identity_bridge_is_deterministic_and_domain_separated() -> Result<(), String> {
    let first = service_process_lifecycle_identity::lease_proof(
        "instance-1",
        "harness",
        "session-1",
        "lease-1",
        1,
    );
    let second = service_process_lifecycle_identity::lease_proof(
        "instance-1",
        "harness",
        "session-1",
        "lease-1",
        1,
    );
    assert_eq!(first, second, "the mapping must be stable across calls");
    assert_eq!(
        service_process_lifecycle_identity::instance_id("instance-1"),
        first.instance_id()
    );
    assert_ne!(
        service_process_lifecycle_identity::instance_id("instance-1"),
        service_process_lifecycle_identity::instance_id("instance-2"),
        "distinct configured instances must map to distinct lifecycle instances"
    );
    assert_ne!(
        first.caller_id(),
        service_process_lifecycle_identity::lease_proof(
            "instance-1", "other", "session-1", "lease-1", 1
        )
        .caller_id(),
        "distinct callers must not collide"
    );
    assert_ne!(
        first.epoch(),
        service_process_lifecycle_identity::lease_proof(
            "instance-1", "harness", "session-1", "lease-1", 2
        )
        .epoch(),
        "the configured lease epoch must reach the proof"
    );
    Ok(())
}

#[test]
fn configured_deployment_reports_adapter_absence_rather_than_silent_success() -> Result<(), String> {
    let deployment = service_process_lifecycle_config::ProcessLifecycleDeployment {
        profiles: process_lifecycle_fixtures::profiles()?,
        max_processes: 4,
        max_records: 16,
        store_path: process_lifecycle_fixtures::store_path(),
    };
    let runtime = service_process_lifecycle::ProcessLifecycleRuntime::configured(&deployment)
        ?;
    assert!(!runtime.is_ready());
    assert_eq!(runtime.profiles(), Some(&[7_u64, 9][..]));
    Ok(())
}

#[test]
fn invalid_deployment_configuration_fails_closed() -> Result<(), String> {
    assert!(service_process_lifecycle_config::parse_profiles("").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("7:11:12:13:14:4:5000").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("0:11:12:13:14:4:5000:5000").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("7:0:12:13:14:4:5000:5000").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("7:11:12:13:0:4:5000:5000").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("7:11:12:13:14:0:5000:5000").is_err());
    assert!(service_process_lifecycle_config::parse_profiles("7:11:12:13:14:4:0:5000").is_err());
    assert!(
        service_process_lifecycle_config::parse_profiles("7:11:12:13:14:4:5000:5000;7:21:22:23:24:4:5000:5000")
            .is_err(),
        "duplicate profile ids must be rejected"
    );
    Ok(())
}

