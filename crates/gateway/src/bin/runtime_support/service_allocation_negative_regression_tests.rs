// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use rusqlite::Connection;
use sts2_gateway::{RecoveryHostLeaseState, RecoveryLeaseRequest};
use uuid::Uuid;

use super::super::host_lease_control::HostLeaseKind;
use super::host_lease_helpers::grant_value;
use super::*;

#[path = "service_allocation_negative_test_support.rs"]
mod support;
#[path = "service_allocation_stop_provenance_tests.rs"]
mod stop_provenance;
use self::support::{
    allocation_body, recovery_revoke_request, retire_fixture_lease, runtime_request_for,
    spawn_signed_ack_server,
};

#[test]
fn confirmed_signed_revoke_reopens_a_fresh_allocation_epoch() -> Result<(), String> {
    let (mut service, old_lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    retire_fixture_lease(&mut service, &old_lease)?;
    let allocation = allocation_body(&service)?;

    let (address, first_server) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install],
        false,
        None,
    )?;
    service.config.mod_address = address;
    let installed = service.allocate(&allocation);
    first_server
        .join()
        .map_err(|_| String::from("initial host fake panicked"))??;
    if installed.0 != 200 {
        super::runtime_v3_catalog_tests::cleanup(service, &path);
        return Err(format!(
            "initial install failed: {} {}",
            installed.0,
            String::from_utf8_lossy(&installed.1)
        ));
    }
    let lease = service
        .recovery_lease
        .clone()
        .ok_or_else(|| String::from("initial lease missing"))?;

    let blocker = Connection::open(&path).map_err(|error| error.to_string())?;
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| error.to_string())?;
    let mut substituted = lease.clone();
    substituted.lease_id = Uuid::new_v4().to_string();
    let failed = super::lease::allocation_response(&mut service, &substituted);
    blocker
        .execute_batch("ROLLBACK")
        .map_err(|error| error.to_string())?;
    assert_eq!(failed.0, 409);
    assert!(service.lease_revoked);
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Installed);

    let untouched = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| error.to_string())?;
    untouched.set_nonblocking(true).map_err(|error| error.to_string())?;
    service.config.mod_address = untouched.local_addr().map_err(|error| error.to_string())?.to_string();
    assert_eq!(service.allocate(b"{}").0, 409);
    let mut wrong: serde_json::Value = serde_json::from_slice(&allocation)
        .map_err(|error| error.to_string())?;
    wrong["caller_id"] = serde_json::Value::String(Uuid::new_v4().to_string());
    assert_eq!(service.allocate(&serde_json::to_vec(&wrong)
        .map_err(|error| error.to_string())?).0, 409);
    assert!(matches!(untouched.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    assert_eq!(service.allocation_cleanup_lease_id.as_deref(), Some(lease.lease_id.as_str()));
    drop(untouched);

    let (address, retry_server) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke, HostLeaseKind::Install],
        true,
        None,
    )?;
    service.config.mod_address = address;
    let fresh = service.allocate(&allocation);
    retry_server
        .join()
        .map_err(|_| String::from("revoke retry host fake panicked"))??;

    assert!(
        !service.lease_revoked,
        "confirmed cleanup left lease_revoked=true; fresh allocation status was {}",
        fresh.0
    );
    assert_eq!(fresh.0, 200);
    let fresh_lease = service
        .recovery_lease
        .as_ref()
        .ok_or_else(|| String::from("fresh lease missing"))?;
    assert_ne!(fresh_lease.lease_id, lease.lease_id);
    assert!(fresh_lease.lease_epoch > lease.lease_epoch);
    let old_binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("old host binding missing"))?;
    assert_eq!(old_binding.state, RecoveryHostLeaseState::HostRevoked);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn expired_install_after_durable_ack_does_not_leave_an_active_installed_lease()
-> Result<(), String> {
    let (mut service, old_lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    retire_fixture_lease(&mut service, &old_lease)?;
    let boot = service
        .recovery_boot
        .clone()
        .ok_or_else(|| String::from("boot missing"))?;
    let fence = service
        .recovery_fence
        .clone()
        .ok_or_else(|| String::from("fence missing"))?;
    let now = service.recovery_now_millis();
    let lease = service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id: boot.deployment_id.clone(),
            instance_id: boot.instance_id.clone(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id: service.config.caller_id.clone(),
            session_id: service.config.session_id.clone(),
            now_millis: now,
            ttl_seconds: service.config.recovery_ttl_seconds,
            renewal_interval_seconds: service.config.recovery_renewal_interval_seconds,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant = grant_value(
        &boot,
        &fence,
        &lease,
        &service.config.caller_id,
        &service.config.session_id,
    );
    let digest = super::super::host_lease_control::grant_digest(&grant)
        .map_err(|error| format!("grant digest: {error:?}"))?;
    service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &digest,
            &fence.host_fence_id,
            fence.fence_generation,
            now,
        )
        .map_err(|error| error.to_string())?;
    service.recovery_lease = Some(lease.clone());
    service.recovery_host_grant = Some(HostLeaseGrant {
        installation_id: installation_id.clone(),
        grant_digest: digest.clone(),
        grant: grant.clone(),
    });
    service.lease_active = false;
    service.lease_revoked = false;
    service.recovery_lease_deadline = Some(Instant::now() + Duration::from_secs(30));
    service.recovery_lease_deadline_lease_id = Some(lease.lease_id.clone());

    let (address, server) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install, HostLeaseKind::Revoke],
        true,
        None,
    )?;
    service.config.mod_address = address;
    super::allocation_cleanup::inject_expiry_after_install();
    let response = service.allocate(&allocation_body(&service)?);
    assert!(
        !super::allocation_cleanup::expiry_fault_pending(),
        "expiry must be injected only after the durable install commits"
    );
    server
        .join()
        .map_err(|_| String::from("expiry host fake panicked"))??;

    assert_eq!(response.0, 410);
    assert!(!service.lease_active);
    assert!(!service.lease_revoked);
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_ne!(
        binding.state,
        RecoveryHostLeaseState::Installed,
        "post-install activation expiry left an ACTIVE/INSTALLED durable host authority; response status was {}",
        response.0
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn allocation_rejects_an_installed_binding_with_a_noncanonical_grant_digest()
-> Result<(), String> {
    let (mut service, lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    let boot = service
        .recovery_boot
        .clone()
        .ok_or_else(|| String::from("boot missing"))?;
    let fence = service
        .recovery_fence
        .clone()
        .ok_or_else(|| String::from("fence missing"))?;
    let grant = grant_value(
        &boot,
        &fence,
        &lease,
        &service.config.caller_id,
        &service.config.session_id,
    );
    let canonical_digest = super::super::host_lease_control::grant_digest(&grant)
        .map_err(|error| format!("grant digest: {error:?}"))?;
    let fault = Connection::open(&path).map_err(|error| error.to_string())?;
    assert_eq!(
        fault.execute(
            "UPDATE leases SET host_grant_digest = ?1 WHERE lease_id = ?2",
            rusqlite::params!["a".repeat(64), lease.lease_id],
        ).map_err(|error| error.to_string())?,
        1,
    );
    drop(fault);
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Installed);
    assert_ne!(
        binding.grant_digest.as_deref(),
        Some(canonical_digest.as_str()),
        "fixture must contain a durable digest mismatch"
    );

    let check = service.check_lease(&runtime_request_for(&service, &lease));
    assert!(check.is_err(), "ordinary admission must reject before cleanup");
    assert!(
        !service.host_lease_ready(&lease.lease_id).map_err(|error| error.to_string())?,
        "sideband admission must reject before cleanup"
    );
    let allocation = super::allocation_context::allocation_response(&mut service, &lease);
    assert_ne!(
        allocation.0,
        200,
        "an installed host grant with a mismatched digest must fail closed; durable={:?}, canonical={canonical_digest}",
        binding.grant_digest
    );
    assert!(
        check.is_err(),
        "ordinary lease admission must validate the canonical host grant digest"
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
