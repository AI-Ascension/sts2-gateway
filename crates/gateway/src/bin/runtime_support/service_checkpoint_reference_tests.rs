// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

const PATH: &str = "/v1/instances/instance-1/checkpoint-reference";

#[test]
fn checkpoint_reference_authority_rejections_never_forward() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    trap.set_nonblocking(true).map_err(|e| e.to_string())?;
    for (header, value, status) in [
        ("authorization", "Bearer invalid", 401),
        ("x-sts2-instance-id", "foreign", 409),
        ("x-sts2-caller-id", "foreign", 409),
        ("x-sts2-session-id", "foreign", 409),
        ("x-mcp-session-id", "foreign", 409),
        ("x-sts2-lease-id", "foreign", 409),
        ("x-sts2-lease-epoch", "2", 409),
        ("x-sts2-correlation-id", "", 400),
    ] {
        let mut service = test_service()?;
        service.config.mod_address = trap.local_addr().map_err(|e| e.to_string())?.to_string();
        let mut request = authenticated_request(PATH);
        request.headers.insert(header.into(), value.into());
        assert_eq!(service.handle_request(&request).0, status, "{header}");
    }
    for scope in ["read", "mutate", "control"] {
        let mut service = test_service()?;
        service.config.mod_address = trap.local_addr().map_err(|e| e.to_string())?.to_string();
        service.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", None, None, scope)?;
        let mut request = authenticated_request(PATH);
        request.body = b"{}".to_vec();
        assert_eq!(
            service.handle_request(&request).0,
            if scope == "read" { 400 } else { 403 }
        );
    }
    for state in ["inactive", "revoked", "shutdown"] {
        let mut service = test_service()?;
        service.config.mod_address = trap.local_addr().map_err(|e| e.to_string())?.to_string();
        service.lease_active = state != "inactive";
        service.lease_revoked = state == "revoked";
        service.shutdown_requested = state == "shutdown";
        assert_eq!(service.handle_request(&authenticated_request(PATH)).0, 409);
    }
    let mut service = test_service()?;
    for path in [
        "/v1/instances/foreign/checkpoint-reference",
        "/v1/instances/instance-1/checkpoint-reference?path=/secret",
    ] {
        assert_eq!(service.handle_request(&authenticated_request(path)).0, 404);
    }
    let mut request = authenticated_request(PATH);
    request.method = "POST".into();
    assert_eq!(service.handle_request(&request).0, 404);
    assert!(matches!(trap.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    Ok(())
}

fn response() -> Result<Value, String> {
    let reference: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/exact-checkpoint-reference-v1/golden/reference.json"
    )))
    .map_err(|e| e.to_string())?;
    Ok(
        json!({"schema":"ascension.checkpoint_reference_response.v1", "instance_id":"instance-1",
        "caller_id":"harness", "session_id":"session-1", "lease_id":"lease-1", "lease_epoch":1,
        "correlation_id":"corr-state", "reference":reference}),
    )
}

#[test]
fn checkpoint_reference_production_dispatch_validates_and_forwards() -> Result<(), String> {
    let encoded = serde_json::to_vec(&response()?).map_err(|e| e.to_string())?;
    let (status, body, request) = exchange(encoded.clone(), 200)?;
    assert_eq!(status, 200);
    assert_eq!(body, encoded);
    assert_eq!(request.path, "/api/checkpoint/v1/reference");
    assert_eq!(request.method, "GET");
    assert!(request.body.is_empty());
    for (key, value) in [
        ("authorization", "Bearer mod-token"),
        ("x-sts2-instance-id", "instance-1"),
        ("x-sts2-caller-id", "harness"),
        ("x-sts2-session-id", "session-1"),
        ("x-sts2-lease-id", "lease-1"),
        ("x-sts2-lease-epoch", "1"),
        ("x-sts2-correlation-id", "corr-state"),
    ] {
        assert_eq!(request.headers.get(key).map(String::as_str), Some(value));
    }
    Ok(())
}

#[test]
fn checkpoint_reference_redacts_invalid_and_unavailable_producers() -> Result<(), String> {
    for field in [
        "instance_id",
        "caller_id",
        "session_id",
        "lease_id",
        "lease_epoch",
        "correlation_id",
        "schema",
        "extra",
    ] {
        let mut value = response()?;
        value[field] = json!("secret");
        let (status, body, _) =
            exchange(serde_json::to_vec(&value).map_err(|e| e.to_string())?, 200)?;
        assert_eq!(status, 502, "{field}");
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
    }
    let mut value = response()?;
    value["reference"]["exact_state_digest"] = json!("secret");
    assert_eq!(
        exchange(serde_json::to_vec(&value).map_err(|e| e.to_string())?, 200)?.0,
        502
    );
    for bytes in [vec![b' '; 8193], b"{\"schema\":1,\"schema\":2}".to_vec()] {
        assert_eq!(exchange(bytes, 200)?.0, 502);
    }
    for upstream in [404, 501, 503] {
        let (status, body, _) = exchange(b"secret".to_vec(), upstream)?;
        assert_eq!(status, 503);
        let value: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        assert_eq!(value["error_code"], "checkpoint_reference_unavailable");
        assert!(value.get("reference").is_none());
    }
    Ok(())
}

fn exchange(response: Vec<u8>, status: u16) -> Result<(u16, Vec<u8>, HttpRequest), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let worker = thread::spawn(move || -> Result<HttpRequest, String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        let request = read_request(&mut stream).map_err(|e| format!("{e:?}"))?;
        // An over-budget legacy consumer may close after reading response headers.
        let _ = write_response(&mut stream, status, &response);
        Ok(request)
    });
    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
    let (status, body) = service.handle_request(&authenticated_request(PATH));
    let request = worker
        .join()
        .map_err(|_| "synthetic downstream panicked".to_owned())??;
    Ok((status, body, request))
}
