// SPDX-License-Identifier: MIT

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{Value, json};
use sts2_gateway::{RecoveryHostLeaseState, RecoveryLease, RecoveryLeaseRequest};
use uuid::Uuid;

use super::super::host_lease_control::HostLeaseKind;
use super::host_lease_helpers::grant_value;
use super::super::recovery_frame::{RecoveryKind, request_frame};
use super::test_support::authenticated_request;
use super::*;

const TEST_CALLER: &str = "00000000-0000-4000-8000-000000000008";
const TEST_SESSION: &str = "00000000-0000-4000-8000-000000000007";

/// A bounded signed host fake.  The optional final request lets the same test
/// observe both the unfixed path (no cleanup/fresh install) and an eventual
/// repaired path without making the unfixed run hang for two seconds.
fn spawn_signed_ack_server(
    key: Vec<u8>,
    principal: String,
    kinds: Vec<HostLeaseKind>,
    optional_last: bool,
    lock: Option<(PathBuf, Duration)>,
) -> Result<(String, thread::JoinHandle<Result<(), String>>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let kind_count = kinds.len();
        for (index, kind) in kinds.into_iter().enumerate() {
            let timeout = if optional_last && index + 1 == kind_count {
                Duration::from_millis(300)
            } else {
                Duration::from_secs(2)
            };
            let Some(mut stream) = accept_with_timeout(&listener, timeout)? else {
                return Ok(());
            };
            let request = super::super::http::read_request(&mut stream)
                .map_err(|status| format!("host request read failed with {status}"))?;
            let request = super::super::host_lease_control::parse_request(&request.body, kind)
                .map_err(|error| format!("host request parse failed: {error:?}"))?;
            let payload = request["payload"].clone();
            let grant = payload["grant"].clone();

            let mut blocker = None;
            if index == 0
                && let Some((path, hold_for)) = lock.as_ref()
            {
                let connection = Connection::open(path).map_err(|error| {
                    format!("host fake could not open allocation store for lock: {error}")
                })?;
                connection
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(|error| format!("host fake could not hold allocation lock: {error}"))?;
                blocker = Some((connection, *hold_for));
            }

            let mut response = json!({
                "contract": sts2_gateway::HOST_LEASE_CONTROL_CONTRACT,
                "schema_digest": sts2_gateway::HOST_LEASE_CONTROL_SCHEMA_DIGEST,
                "message_id": Uuid::new_v4().to_string(),
                "correlation_id": request["correlation_id"].clone(),
                "sent_at": request["sent_at"].clone(),
                "actor": {"principal_id": principal.clone(), "role": "host"},
                "auth": {
                    "principal_id": principal.clone(),
                    "capability": kind.capability(),
                    "proof": "placeholder"
                },
                "kind": kind.response_name(),
                "payload": {
                    "ack": {
                        "result": {
                            "status": if kind == HostLeaseKind::Revoke { "REVOKED" } else { "INSTALLED" },
                            "retryable": false,
                            "retry_after_seconds": null
                        },
                        "installation_id": payload["installation_id"].clone(),
                        "grant_digest": payload["grant_digest"].clone(),
                        "boot_id": grant["boot"]["boot_id"].clone(),
                        "instance_incarnation": grant["boot"]["instance_incarnation"].clone(),
                        "host_fence_id": grant["fence"]["host_fence_id"].clone(),
                        "fence_generation": grant["fence"]["fence_generation"].clone(),
                        "lease_id": grant["lease"]["lease_id"].clone(),
                        "lease_epoch": grant["lease"]["lease_epoch"].clone(),
                        "host_install_generation": 1,
                        "recorded_at": request["sent_at"].clone(),
                        "renew_sequence": null,
                        "expires_at": if kind == HostLeaseKind::Revoke {
                            Value::Null
                        } else {
                            grant["lease"]["expires_at"].clone()
                        }
                    }
                }
            });
            let proof = super::super::host_lease_control_crypto::proof_for_frame(
                &response,
                kind.acknowledgment_domain(),
                &key,
            );
            response["auth"]["proof"] = Value::String(proof);
            let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
            super::super::host_lease_control::parse_response(
                &body,
                kind,
                &key,
                response["actor"]["principal_id"]
                    .as_str()
                    .ok_or_else(|| String::from("response principal missing"))?,
            )
            .map_err(|error| format!("constructed host response rejected: {error:?}"))?;
            super::super::http::write_response(&mut stream, 200, &body)
                .map_err(|error| error.to_string())?;

            if let Some((connection, hold_for)) = blocker {
                thread::sleep(hold_for);
                connection
                    .execute_batch("ROLLBACK")
                    .map_err(|error| format!("host fake could not release allocation lock: {error}"))?;
            }
        }
        Ok(())
    });
    Ok((address.to_string(), server))
}

fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> Result<Option<TcpStream>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(Some(stream)),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(format!("host request accept failed: {error}")),
        }
    }
}

fn retire_fixture_lease(
    service: &mut RuntimeService,
    old_lease: &RecoveryLease,
) -> Result<(), String> {
    service.config.caller_id = TEST_CALLER.to_owned();
    service.config.session_id = TEST_SESSION.to_owned();
    service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?
        .revoke_lease(&old_lease.proof(), "test_cleanup")
        .map_err(|error| error.to_string())?;
    service.recovery_lease = None;
    service.recovery_lease_deadline = None;
    service.recovery_lease_deadline_lease_id = None;
    service.recovery_host_grant = None;
    service.lease_active = false;
    service.lease_revoked = false;
    Ok(())
}

fn allocation_body(service: &RuntimeService) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "instance_id": service.config.instance_id,
        "caller_id": service.config.caller_id,
        "session_id": service.config.session_id,
    }))
    .map_err(|error| error.to_string())
}

fn recovery_revoke_request(
    service: &RuntimeService,
    lease: &RecoveryLease,
) -> HttpRequest {
    let correlation = Uuid::new_v4().to_string();
    let mut request = authenticated_request("/v1/recovery/lease/revoke");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.headers.insert(
        String::from("x-sts2-instance-id"),
        service.config.instance_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-caller-id"),
        service.config.caller_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-session-id"),
        service.config.session_id.clone(),
    );
    request
        .headers
        .insert(String::from("x-sts2-lease-id"), lease.lease_id.clone());
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), correlation.clone());
    request.headers.insert(
        String::from("x-sts2-recovery-capability"),
        String::from("lease_revoke"),
    );
    request.body = request_frame(
        RecoveryKind::LeaseRevoke,
        &service.config.caller_id,
        &correlation,
        None,
        json!({
            "lease": super::recovery_wire::lease_value(lease),
            "reason": "shutdown",
        }),
    );
    request
}

fn runtime_request_for(
    service: &RuntimeService,
    lease: &RecoveryLease,
) -> HttpRequest {
    let mut request = authenticated_request("/v1/instances/unused/action");
    request.headers.insert(
        String::from("x-sts2-instance-id"),
        lease.instance_id.clone(),
    );
    request
        .headers
        .insert(String::from("x-sts2-caller-id"), service.config.caller_id.clone());
    request
        .headers
        .insert(String::from("x-sts2-session-id"), service.config.session_id.clone());
    request
        .headers
        .insert(String::from("x-sts2-lease-id"), lease.lease_id.clone());
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    request
}

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

    let (address, retry_server) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke, HostLeaseKind::Install],
        true,
        None,
    )?;
    service.config.mod_address = address;
    let revoke = service.handle_request(&recovery_revoke_request(&service, &lease));
    let fresh = service.allocate(&allocation);
    retry_server
        .join()
        .map_err(|_| String::from("revoke retry host fake panicked"))??;

    assert_eq!(revoke.0, 200);
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
    service.recovery_lease_deadline = Some(Instant::now() + Duration::from_millis(250));
    service.recovery_lease_deadline_lease_id = Some(lease.lease_id.clone());

    let (address, server) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install, HostLeaseKind::Revoke],
        true,
        Some((path.clone(), Duration::from_millis(750))),
    )?;
    service.config.mod_address = address;
    let response = service.allocate(&allocation_body(&service)?);
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

    let allocation = super::allocation_context::allocation_response(&mut service, &lease);
    let check = service.check_lease(&runtime_request_for(&service, &lease));
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
