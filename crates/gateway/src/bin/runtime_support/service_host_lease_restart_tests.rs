// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::RecoveryHostLeaseState;

use super::runtime_v3_catalog_tests::{cleanup, recovery_service, runtime_request};

#[test]
fn persisted_install_without_current_process_grant_stays_historical() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    service.recovery_host_grant = None;

    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Installed);
    assert!(!service
        .host_lease_ready(&lease.lease_id)
        .map_err(|error| error.to_string())?);

    let request = runtime_request(
        &service,
        &lease,
        "state",
        json!({"correlation_id": "restart-guard"}),
    )?;
    let (status, body) = service
        .check_lease(&request)
        .err()
        .ok_or_else(|| String::from("historical install admitted mutation"))?;
    assert_eq!(status, 503);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body)
            .map_err(|error| error.to_string())?["error_code"],
        "recovery_host_lease_required"
    );

    let boot = service
        .recovery_boot
        .clone()
        .ok_or_else(|| String::from("recovery boot missing"))?;
    let fence = service
        .recovery_fence
        .clone()
        .ok_or_else(|| String::from("recovery fence missing"))?;
    assert!(service
        .prepare_host_install(&boot, &fence, &lease)
        .is_err());
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Installed);
    cleanup(service, &path);
    Ok(())
}
