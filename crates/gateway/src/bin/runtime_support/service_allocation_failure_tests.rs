// SPDX-License-Identifier: MIT

use super::runtime_v3_catalog_tests::{cleanup, recovery_service};
use super::test_support::authenticated_request;
use super::*;
use sts2_gateway::{RecoveryHostLeaseState, RecoveryLease};

fn request_for(lease: &RecoveryLease) -> HttpRequest {
    let mut request = authenticated_request("/v3/instances/unused/action");
    request
        .headers
        .insert("x-sts2-instance-id".into(), lease.instance_id.clone());
    request
        .headers
        .insert("x-sts2-lease-id".into(), lease.lease_id.clone());
    request
        .headers
        .insert("x-sts2-lease-epoch".into(), lease.lease_epoch.to_string());
    request
}

#[test]
fn allocation_rejects_substituted_lease_and_revokes_actual_lease() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let mut substituted = lease.clone();
    substituted.lease_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        super::lease::allocation_response(&mut service, &substituted).0,
        409
    );
    assert!(!service.lease_active);
    assert!(service.lease_revoked);
    assert!(service.check_lease(&request_for(&lease)).is_err());
    let binding = service
        .recovery
        .as_ref()
        .ok_or("store missing")?
        .host_lease_binding(&lease.lease_id)
        .map_err(|e| e.to_string())?
        .ok_or("binding missing")?;
    assert_eq!(binding.state, RecoveryHostLeaseState::PendingHostRevoke);
    cleanup(service, &path);
    Ok(())
}

#[test]
fn allocation_failure_cleans_durable_lease_even_when_local_active_is_false() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    service.lease_active = false;
    assert_eq!(
        super::lease::allocation_response(&mut service, &lease).0,
        503
    );
    let now = service.recovery_now_millis();
    assert!(
        service
            .recovery
            .as_mut()
            .ok_or("store missing")?
            .validate_lease(&lease.proof(), now)
            .is_err()
    );
    assert!(service.check_lease(&request_for(&lease)).is_err());
    cleanup(service, &path);
    Ok(())
}

#[test]
fn allocation_failure_quarantines_when_durable_revocation_is_busy() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let blocker = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let fence = service.recovery_fence.take();
    let response = super::lease::allocation_response(&mut service, &lease);
    blocker
        .execute_batch("ROLLBACK")
        .map_err(|e| e.to_string())?;
    assert_eq!(response.0, 503);
    assert!(service.lease_revoked);
    let now = service.recovery_now_millis();
    // Prove the write failed: storage alone still admits this proof after the lock ends.
    service
        .recovery
        .as_mut()
        .ok_or("store missing")?
        .validate_lease(&lease.proof(), now)
        .map_err(|e| e.to_string())?;
    assert!(service.check_lease(&request_for(&lease)).is_err());
    assert_eq!(service.allocate(&[]).0, 409);
    service.recovery_fence = fence;
    assert_recovery_mutation_routes_closed(&mut service, &lease)?;
    drop(blocker);
    cleanup(service, &path);
    Ok(())
}

fn assert_recovery_mutation_routes_closed(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
) -> Result<(), String> {
    use super::super::recovery_frame::{RecoveryFrame, RecoveryKind, request_frame};
    service.config.caller_id = "00000000-0000-4000-8000-000000000008".into();
    let lease = super::recovery_wire::lease_value(lease);
    let boot =
        super::recovery_wire::boot_value(service.recovery_boot.as_ref().ok_or("boot missing")?);
    let fence =
        super::recovery_wire::fence_value(service.recovery_fence.as_ref().ok_or("fence missing")?);
    // Operation content is deliberately not decoded: closed admission must
    // precede operation parsing, durable intent, and all downstream work.
    for (kind, payload) in [
        (
            RecoveryKind::LeaseAcquire,
            json!({"boot": boot, "fence": fence}),
        ),
        (
            RecoveryKind::LeaseRenew,
            json!({"lease": lease, "renew_sequence": 1}),
        ),
        (
            RecoveryKind::OperationIntent,
            json!({"lease": lease, "operation": null}),
        ),
        (
            RecoveryKind::OperationDispatch,
            json!({"lease": lease, "operation": null}),
        ),
    ] {
        let mut request = authenticated_request("/v1/recovery/unused");
        request
            .headers
            .insert("content-type".into(), "application/json".into());
        request.headers.insert(
            "x-sts2-recovery-capability".into(),
            kind.capability().into(),
        );
        request.body = request_frame(
            kind,
            &service.config.caller_id,
            &uuid::Uuid::new_v4().to_string(),
            None,
            payload,
        );
        RecoveryFrame::parse(&request.body, kind).map_err(|e| format!("test frame: {e:?}"))?;
        let (status, body) = service.recovery_route(&request, kind);
        assert_eq!(status, 409);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string())?["error_code"],
            "lease_context_revoked"
        );
    }
    Ok(())
}

#[test]
fn allocation_response_rejects_elapsed_monotonic_deadline() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    service.recovery_lease_deadline = Some(Instant::now() - Duration::from_millis(1));
    assert_ne!(
        super::lease::allocation_response(&mut service, &lease).0,
        200
    );
    assert!(service.lease_revoked);
    assert!(service.check_lease(&request_for(&lease)).is_err());
    cleanup(service, &path);
    Ok(())
}
