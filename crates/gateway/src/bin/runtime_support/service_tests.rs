// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

#[test]
fn v3_envelope_is_validated_before_any_downstream_connection() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v3/instances/instance-1/action");
    request.method = "POST".to_owned();
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = b"{}".to_vec();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 400);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "runtime_v3_request_invalid"
    );
    Ok(())
}

#[test]
fn allocation_rejects_duplicate_unknown_and_missing_members() -> Result<(), String> {
    for body in [
        br#"{"instance_id":"wrong","instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#.as_slice(),
        br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1","extra":true}"#,
        br#"{"instance_id":"instance-1","caller_id":"harness"}"#,
    ] {
        let mut service = test_service()?;
        service.lease_active = false;
        assert_eq!(service.allocate(body).0, 400);
        assert!(!service.lease_active);
    }
    let mut service = test_service()?;
    service.lease_active = false;
    assert_eq!(
        service
            .allocate(br#"{"instance_id":"wrong","caller_id":"harness","session_id":"session-1"}"#)
            .0,
        409
    );
    assert!(!service.lease_active);
    assert_eq!(
        service
            .allocate(
                br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#
            )
            .0,
        200
    );
    assert!(service.lease_active);
    Ok(())
}

#[test]
fn recovery_allocation_response_binds_the_acquired_lease_and_current_fence() -> Result<(), String> {
    let (service, lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    let (status, body) = super::lease::allocation_response(&service, &lease);
    assert_eq!(status, 200);
    let response: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    let lease_id = response["lease_id"]
        .as_str()
        .ok_or_else(|| String::from("lease_id missing"))?;
    assert_eq!(response["lease_id"], lease.lease_id);
    assert_eq!(response["lease_epoch"], lease.lease_epoch);
    assert_ne!(response["lease_id"], service.config.lease_id);
    assert!(
        service
            .recovery
            .as_ref()
            .ok_or_else(|| String::from("recovery store missing"))?
            .host_lease_is_ready(lease_id)
            .map_err(|error| error.to_string())?
    );
    assert_eq!(response["fence_token"], lease.fence_token);
    let authority = &response["recovery_authority"];
    assert_eq!(authority["contract"], "watchdog-runtime-allocation-v1");
    assert_eq!(
        authority["schema_digest"],
        super::allocation_context::ALLOCATION_SCHEMA_DIGEST
    );
    assert_eq!(
        authority["context"],
        serde_json::json!({
            "deployment_id": lease.deployment_id,
            "instance_id": lease.instance_id,
            "instance_incarnation": lease.instance_incarnation,
            "boot_id": lease.boot_id,
            "authority_generation": lease.authority_generation,
            "lease_id": lease.lease_id,
            "lease_epoch": lease.lease_epoch,
        })
    );
    let fence = service
        .recovery_fence
        .as_ref()
        .ok_or_else(|| String::from("recovery fence missing"))?;
    assert_eq!(
        authority["current_fence"],
        super::recovery_wire::fence_value(fence)
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn recovery_allocation_response_fails_closed_without_matching_fence() -> Result<(), String> {
    let (mut service, lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    service.recovery_fence = None;
    let (status, body) = super::lease::allocation_response(&service, &lease);
    assert_eq!(status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "recovery_host_fence_required"
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);

    let (mut service, lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    service
        .recovery_fence
        .as_mut()
        .ok_or_else(|| String::from("recovery fence missing"))?
        .boot_id = String::from("00000000-0000-4000-8000-000000000099");
    let (status, body) = super::lease::allocation_response(&service, &lease);
    assert_eq!(status, 503);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "recovery_allocation_authority_mismatch"
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn recovery_allocate_route_fails_closed_before_success_without_current_fence() -> Result<(), String>
{
    let (mut service, _lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    service.recovery_fence = None;
    let (status, _body) = service.allocate(
        br#"{"instance_id":"00000000-0000-4000-8000-000000000002","caller_id":"harness","session_id":"session-1"}"#,
    );
    assert_eq!(status, 503);
    super::runtime_v3_catalog_tests::cleanup(service, &path);

    let (mut service, _lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    service
        .recovery_fence
        .as_mut()
        .ok_or_else(|| String::from("recovery fence missing"))?
        .boot_id = String::from("00000000-0000-4000-8000-000000000099");
    let (status, _body) = service.allocate(
        br#"{"instance_id":"00000000-0000-4000-8000-000000000002","caller_id":"harness","session_id":"session-1"}"#,
    );
    assert_ne!(status, 200);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn nonrecovery_allocation_response_remains_unchanged() -> Result<(), String> {
    let mut service = test_service()?;
    let (status, body) = service.allocate(
        br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?,
        serde_json::json!({
            "status": "allocated",
            "instance_id": "instance-1",
            "caller_id": "harness",
            "session_id": "session-1",
            "lease_id": "lease-1",
            "lease_epoch": 1,
            "transport": "attached-loopback"
        })
    );
    Ok(())
}

#[test]
fn v1_fixed_forwarding_preserves_paths_credentials_and_identity() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(3);
    let worker = thread::spawn(move || -> Result<(), String> {
        for _ in 0..3 {
            let expires = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < expires =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            let request = read_request(&mut stream).map_err(|e| e.to_string())?;
            sender.send(request).map_err(|e| e.to_string())?;
            write_response(&mut stream, 200, br#"{"synthetic":true}"#)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    });
    let mut service = test_service()?;
    service.config.mod_address = address.to_string();
    for (method, public_path, downstream_path, body) in [
        ("GET", "/health/ready", "/health/ready", b"".as_slice()),
        (
            "GET",
            "/v1/instances/instance-1/state",
            "/api/v1/runtime/state",
            b"",
        ),
        (
            "POST",
            "/v1/instances/instance-1/action",
            "/api/v1/runtime/action",
            br#"{"action":"synthetic"}"#,
        ),
    ] {
        let mut request = authenticated_request(public_path);
        request.method = method.to_owned();
        request.body = body.to_vec();
        if !body.is_empty() {
            request
                .headers
                .insert("content-type".to_owned(), "application/json".to_owned());
        }
        let (status, response) = service.handle_request(&request);
        assert_eq!(status, 200);
        if public_path != "/health/ready" {
            assert_eq!(response, br#"{"synthetic":true}"#);
        }
        let forwarded = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        assert_eq!(forwarded.method, method);
        assert_eq!(forwarded.path, downstream_path);
        assert_eq!(forwarded.body, body);
        assert_eq!(
            forwarded.headers.get("authorization").map(String::as_str),
            Some("Bearer mod-token")
        );
        assert!(!forwarded.headers.contains_key("x-mcp-session-id"));
        if public_path == "/health/ready" {
            assert!(!forwarded.headers.contains_key("x-sts2-correlation-id"));
        } else {
            for (name, expected) in [
                ("x-sts2-instance-id", "instance-1"),
                ("x-sts2-caller-id", "harness"),
                ("x-sts2-session-id", "session-1"),
                ("x-sts2-lease-id", "lease-1"),
                ("x-sts2-lease-epoch", "1"),
                ("x-sts2-correlation-id", "corr-state"),
            ] {
                assert_eq!(
                    forwarded.headers.get(name).map(String::as_str),
                    Some(expected)
                );
            }
        }
    }
    worker
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    Ok(())
}

#[test]
fn host_fence_control_route_bridges_without_a_gameplay_lease() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let worker = thread::spawn(move || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let request = read_request(&mut stream).map_err(|error| error.to_string())?;
        sender.send(request).map_err(|error| error.to_string())?;
        write_response(&mut stream, 200, b"{}").map_err(|error| error.to_string())
    });
    let mut service = test_service()?;
    service.config.mod_address = address.to_string();
    let mut request = authenticated_request("/v1/recovery/host-fence");
    request.method = String::from("POST");
    request.headers.remove("x-sts2-lease-id");
    request.headers.remove("x-sts2-lease-epoch");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = frame();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, b"{}");
    let forwarded = receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(forwarded.method, "POST");
    assert_eq!(forwarded.path, "/api/v1/runtime/recovery");
    assert_eq!(forwarded.body, request.body);
    assert_eq!(
        forwarded.headers.get("authorization").map(String::as_str),
        Some("Bearer mod-token")
    );
    assert!(!forwarded.headers.contains_key("x-sts2-lease-id"));
    assert!(!forwarded.headers.contains_key("x-sts2-lease-epoch"));
    worker
        .join()
        .map_err(|_| String::from("host-fence downstream panicked"))??;
    Ok(())
}

fn frame() -> Vec<u8> {
    format!(
        r#"{{"contract":"watchdog-recovery-v1","schema_digest":"{}","message_id":"00000000-0000-4000-8000-000000000001","correlation_id":"00000000-0000-4000-8000-000000000002","sent_at":"2026-09-06T23:00:00Z","actor":{{"principal_id":"00000000-0000-4000-8000-000000000003","role":"gateway"}},"auth":{{"principal_id":"00000000-0000-4000-8000-000000000003","capability":"host_fence","proof":"proof-1"}},"kind":"host_fence_request","payload":{{"boot":{{"deployment_id":"00000000-0000-4000-8000-000000000004","instance_id":"00000000-0000-4000-8000-000000000005","instance_incarnation":"00000000-0000-4000-8000-000000000006","boot_id":"00000000-0000-4000-8000-000000000007","authority_generation":1,"release":{{"release_digest":"0000000000000000000000000000000000000000000000000000000000000000","config_digest":"0000000000000000000000000000000000000000000000000000000000000000","profile_digest":"0000000000000000000000000000000000000000000000000000000000000000","runtime_v3_schema_digest":"8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63"}},"created_at":"2026-09-06T23:00:00Z","state":"FENCE_REQUIRED"}}}}}}"#,
        sts2_gateway::RECOVERY_SCHEMA_DIGEST
    )
    .into_bytes()
}
