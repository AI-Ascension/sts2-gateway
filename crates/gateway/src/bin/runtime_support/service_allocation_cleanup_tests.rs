// SPDX-License-Identifier: MIT

use super::super::host_lease_control::HostLeaseKind;
use super::test_support::*;
use super::*;

fn spawn_install_ack_server(
    key: Vec<u8>,
    principal: String,
    kinds: Vec<HostLeaseKind>,
) -> Result<(String, thread::JoinHandle<Result<(), String>>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        for kind in kinds {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(format!("host request accept failed: {error}")),
                }
            };
            let request = read_request(&mut stream)
                .map_err(|status| format!("host request read failed with {status}"))?;
            let request = super::super::host_lease_control::parse_request(&request.body, kind)
                .map_err(|error| format!("host request parse failed: {error:?}"))?;
            let payload = request["payload"].clone();
            let grant = payload["grant"].clone();
            let mut response = json!({
                "contract": sts2_gateway::HOST_LEASE_CONTROL_CONTRACT,
                "schema_digest": sts2_gateway::HOST_LEASE_CONTROL_SCHEMA_DIGEST,
                "message_id": uuid::Uuid::new_v4().to_string(),
                "correlation_id": request["correlation_id"].clone(),
                "sent_at": request["sent_at"].clone(),
                "actor": {"principal_id": principal, "role": "host"},
                "auth": {
                    "principal_id": principal,
                    "capability": kind.capability(),
                    "proof": "placeholder"
                },
                "kind": kind.response_name(),
                "payload": {
                    "ack": {
                        "result": {"status": if kind == HostLeaseKind::Revoke { "REVOKED" } else { "INSTALLED" }, "retryable": false, "retry_after_seconds": null},
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
                        "expires_at": if kind == HostLeaseKind::Revoke { Value::Null } else { grant["lease"]["expires_at"].clone() }
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
            write_response(&mut stream, 200, &body).map_err(|error| error.to_string())?;
        }
        Ok(())
    });
    Ok((address.to_string(), server))
}

#[test]
fn recovery_allocate_route_quarantines_after_post_acquisition_fence_mismatch() -> Result<(), String>
{
    let (mut service, old_lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
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
    service.config.caller_id = String::from("00000000-0000-4000-8000-000000000008");
    service.config.session_id = String::from("00000000-0000-4000-8000-000000000007");

    let (address, server) = spawn_install_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install],
    )?;
    service.config.mod_address = address;
    super::allocation_context::inject_post_install_fence_mismatch();
    let mut request = authenticated_request("/v1/sessions/allocate");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = serde_json::to_vec(&json!({
        "instance_id": service.config.instance_id,
        "caller_id": service.config.caller_id,
        "session_id": service.config.session_id,
    }))
    .map_err(|e| e.to_string())?;
    let (status, body) = service.handle_request(&request);
    let server_result = server
        .join()
        .map_err(|_| String::from("host server panicked"))?;
    server_result?;
    assert_eq!(status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "recovery_allocation_authority_mismatch"
    );
    assert!(!service.lease_active);
    assert!(service.lease_revoked);
    let lease = service
        .recovery_lease
        .as_ref()
        .ok_or_else(|| String::from("quarantined lease identity missing"))?;
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?;
    assert_eq!(
        binding.state,
        sts2_gateway::RecoveryHostLeaseState::PendingHostRevoke
    );
    assert_eq!(service.allocate(&[]).0, 409);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn allocation_cleanup_acknowledgment_closes_host_and_preserves_prior_stop() -> Result<(), String> {
    for stopped in [false, true] {
        let (mut service, old_lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
        service
            .recovery
            .as_mut()
            .ok_or("store missing")?
            .revoke_lease(&old_lease.proof(), "test_cleanup")
            .map_err(|e| e.to_string())?;
        service.recovery_lease = None;
        service.recovery_lease_deadline = None;
        service.recovery_lease_deadline_lease_id = None;
        service.recovery_host_grant = None;
        service.lease_active = false;
        service.lease_revoked = false;
        service.config.caller_id = String::from("00000000-0000-4000-8000-000000000008");
        service.config.session_id = String::from("00000000-0000-4000-8000-000000000007");
        let mut kinds = vec![HostLeaseKind::Install, HostLeaseKind::Revoke];
        if !stopped {
            kinds.push(HostLeaseKind::Install);
        }
        let (address, server) = spawn_install_ack_server(
            service.config.host_lease_key.clone(),
            service.config.host_principal_id.clone(),
            kinds,
        )?;
        service.config.mod_address = address;
        let allocation = serde_json::to_vec(&json!({
            "instance_id": service.config.instance_id,
            "caller_id": service.config.caller_id,
            "session_id": service.config.session_id,
        }))
        .map_err(|e| e.to_string())?;
        let installed = service.allocate(&allocation);
        if installed.0 != 200 {
            let server_result = server.join().map_err(|_| "host server panicked")?;
            return Err(format!(
                "install failed {}: {}; server {server_result:?}",
                installed.0,
                String::from_utf8_lossy(&installed.1)
            ));
        }
        let lease = service.recovery_lease.clone().ok_or("lease missing")?;
        service.lease_revoked = stopped;
        let mut substituted = lease.clone();
        substituted.lease_id = uuid::Uuid::new_v4().to_string();
        let response = super::lease::allocation_response(&mut service, &substituted);
        let closed = !service.lease_active && service.recovery_lease.is_none();
        let preserved_stop = service.lease_revoked;
        let fresh = service.allocate(&allocation);
        server.join().map_err(|_| "host server panicked")??;
        assert_eq!(response.0, 409);
        assert!(closed);
        assert_eq!(preserved_stop, stopped);
        if stopped {
            assert_eq!(fresh.0, 409);
            assert!(!service.lease_active);
        } else {
            assert_eq!(fresh.0, 200);
            let fresh = service
                .recovery_lease
                .as_ref()
                .ok_or("fresh lease missing")?;
            assert_ne!(fresh.lease_id, lease.lease_id);
            assert!(fresh.lease_epoch > lease.lease_epoch);
        }
        let binding = service
            .recovery
            .as_ref()
            .ok_or("store missing")?
            .host_lease_binding(&lease.lease_id)
            .map_err(|e| e.to_string())?
            .ok_or("binding missing")?;
        assert_eq!(
            binding.state,
            sts2_gateway::RecoveryHostLeaseState::HostRevoked
        );
        super::runtime_v3_catalog_tests::cleanup(service, &path);
    }
    Ok(())
}
