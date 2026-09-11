// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

const MAP_PATH: &str = "/v1/instances/instance-1/map-snapshot";

#[test]
fn map_rejections_do_not_connect_to_the_downstream() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    trap.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = trap.local_addr().map_err(|e| e.to_string())?.to_string();
    for (header, value, expected) in [
        ("authorization", "Bearer wrong", 401),
        ("x-sts2-instance-id", "other", 409),
        ("x-sts2-caller-id", "other", 409),
        ("x-sts2-session-id", "other", 409),
        ("x-mcp-session-id", "other", 409),
        ("x-sts2-lease-id", "other", 409),
        ("x-sts2-lease-epoch", "2", 409),
        ("x-sts2-correlation-id", "", 400),
    ] {
        let mut service = test_service()?;
        service.config.mod_address.clone_from(&address);
        let mut request = authenticated_request(MAP_PATH);
        request.headers.insert(header.to_owned(), value.to_owned());
        assert_eq!(service.handle_request(&request).0, expected, "{header}");
    }
    for scope in ["read", "mutate", "control"] {
        let mut service = test_service()?;
        service.config.mod_address.clone_from(&address);
        service.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", None, None, scope)?;
        let mut request = authenticated_request(MAP_PATH);
        request.body = b"{}".to_vec();
        assert_eq!(
            service.handle_request(&request).0,
            if scope == "read" { 400 } else { 403 }
        );
    }
    for revoked in [false, true] {
        let mut service = test_service()?;
        service.config.mod_address.clone_from(&address);
        service.lease_active = false;
        service.lease_revoked = revoked;
        assert_eq!(
            service.handle_request(&authenticated_request(MAP_PATH)).0,
            409
        );
    }
    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut wrong_method = authenticated_request(MAP_PATH);
    wrong_method.method = "POST".to_owned();
    assert_eq!(service.handle_request(&wrong_method).0, 404);
    assert!(matches!(trap.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    Ok(())
}

#[test]
fn map_service_forwards_identity_and_accepts_its_larger_budget() -> Result<(), String> {
    let mut response = map_response()?;
    response.resize(MAX_RESPONSE_BYTES + 1, b' ');
    let expected = response.clone();
    let (status, body, request) = exchange(response, false)?;
    assert_eq!(status, 200);
    assert_eq!(body, expected);
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/map/v1/snapshot");
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
fn map_service_rejects_foreign_response_and_legacy_keeps_its_budget() -> Result<(), String> {
    let mut foreign: Value = serde_json::from_slice(&map_response()?).map_err(|e| e.to_string())?;
    foreign["session_id"] = json!("foreign");
    let (status, body, _) = exchange(
        serde_json::to_vec(&foreign).map_err(|e| e.to_string())?,
        false,
    )?;
    assert_eq!(status, 502);
    let error: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    assert_eq!(error["error_code"], "runtime_map_response_invalid");
    let mut oversized = map_response()?;
    oversized.resize(MAX_RESPONSE_BYTES + 1, b' ');
    assert_eq!(exchange(oversized, true)?.0, 502);
    Ok(())
}

#[test]
fn map_service_accepts_exact_multibyte_text_and_timeout_boundaries() -> Result<(), String> {
    for (field, maximum_bytes) in [
        ("game_build", 128_usize),
        ("mod_version", 128),
        ("reason", 256),
    ] {
        let unit = "🦀";
        let mut response: Value =
            serde_json::from_slice(&map_response()?).map_err(|error| error.to_string())?;
        if field == "reason" {
            response["snapshot"]["availability"] = json!("unavailable");
            response["snapshot"]["completeness"] = json!("incomplete");
        }
        response["snapshot"][field] = json!(unit.repeat(maximum_bytes / unit.len()));
        let encoded = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        let (status, body, _) = exchange(encoded.clone(), false)?;
        assert_eq!(status, 200, "{field}");
        assert_eq!(body, encoded, "{field}");
    }

    let mut response: Value =
        serde_json::from_slice(&map_response()?).map_err(|error| error.to_string())?;
    response["timeout"]["timeout_millis"] = json!(120_000);
    response["timeout"]["elapsed_millis"] = json!(120_000);
    let encoded = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    assert_eq!(exchange(encoded.clone(), false)?.0, 200);
    Ok(())
}

#[test]
fn map_service_returns_502_for_text_and_timeout_contract_violations() -> Result<(), String> {
    for (field, maximum_bytes) in [
        ("game_build", 128_usize),
        ("mod_version", 128),
        ("reason", 256),
    ] {
        let unit = "🦀";
        let mut response: Value =
            serde_json::from_slice(&map_response()?).map_err(|error| error.to_string())?;
        if field == "reason" {
            response["snapshot"]["availability"] = json!("unavailable");
            response["snapshot"]["completeness"] = json!("incomplete");
        }
        response["snapshot"][field] =
            json!(format!("{}a", unit.repeat(maximum_bytes / unit.len())));
        let (status, body, _) = exchange(
            serde_json::to_vec(&response).map_err(|error| error.to_string())?,
            false,
        )?;
        assert_eq!(status, 502, "{field} byte overflow");
        let error: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        assert_eq!(
            error["error_code"], "runtime_map_response_invalid",
            "{field}"
        );
    }

    for control in ['\u{0000}', '\u{007f}', '\u{0080}'] {
        let mut response: Value =
            serde_json::from_slice(&map_response()?).map_err(|error| error.to_string())?;
        response["snapshot"]["game_build"] = json!(format!("prefix{control}suffix"));
        let (status, body, _) = exchange(
            serde_json::to_vec(&response).map_err(|error| error.to_string())?,
            false,
        )?;
        assert_eq!(status, 502, "U+{:04X}", control as u32);
        let error: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        assert_eq!(error["error_code"], "runtime_map_response_invalid");
    }

    let mut response: Value =
        serde_json::from_slice(&map_response()?).map_err(|error| error.to_string())?;
    response["timeout"]["timeout_millis"] = json!(1);
    response["timeout"]["elapsed_millis"] = json!(2);
    let (status, body, _) = exchange(
        serde_json::to_vec(&response).map_err(|error| error.to_string())?,
        false,
    )?;
    assert_eq!(status, 502);
    let error: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(error["error_code"], "runtime_map_response_invalid");
    Ok(())
}

fn map_response() -> Result<Vec<u8>, String> {
    let mut response: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-map-v1/golden/snapshot-response.json"
    )))
    .map_err(|e| e.to_string())?;
    response["correlation_id"] = json!("corr-state");
    response["lease_epoch"] = json!(1);
    serde_json::to_vec(&response).map_err(|e| e.to_string())
}

fn exchange(response: Vec<u8>, legacy: bool) -> Result<(u16, Vec<u8>, HttpRequest), String> {
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
        let _ = write_response(&mut stream, 200, &response);
        Ok(request)
    });
    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
    let (status, body) = if legacy {
        match service.forward_mod("GET", "/api/v1/runtime/state", &[], Some("corr-state")) {
            Ok(response) => (response.status, response.body),
            Err(status) => (status, Vec::new()),
        }
    } else {
        service.handle_request(&authenticated_request(MAP_PATH))
    };
    let request = worker
        .join()
        .map_err(|_| "synthetic downstream panicked".to_owned())??;
    Ok((status, body, request))
}
