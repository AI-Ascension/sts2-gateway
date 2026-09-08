// SPDX-License-Identifier: MIT

use super::*;

use super::test_support::*;
use sts2_gateway::RuntimeV2Metadata;

#[test]
fn action_operation_ids_must_be_reachable_by_the_fixed_receipt_route() -> Result<(), String> {
    // No downstream TCP connection may be made for any rejected identity.
    let downstream = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    downstream
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    let address = downstream
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let mut service = test_service()?;
    service
        .runtime_v2
        .forwarding_mut()
        .clone_from(&HttpRuntimeV2Forwarder::new(
            &address,
            "mod-token",
            "instance-1",
            "harness",
            "session-1",
            "lease-1",
            1,
        ));
    for id in ["op/1", "/", "op?1", "op%2f1"] {
        let v2 = RuntimeV2Message::action_request(
            RuntimeV2Metadata::new(),
            "corr-state",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            0,
            id,
            sts2_gateway::RuntimeV2Action::end_turn(),
        );
        let mut request = authenticated_request("/v2/instances/instance-1/action");
        request.method = String::from("POST");
        request.headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        request.body = serde_json::to_vec(&v2).map_err(|e| e.to_string())?;
        let (status, bytes) = service.handle_request(&request);
        assert_eq!(status, 400);
        let response: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        assert_eq!(response["error_code"], "runtime_v2_operation_invalid");
    }
    assert!(
        matches!(downstream.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    for id in ["op-1", "op_1", "op.1", "op:1"] {
        assert!(safe_operation_id(id));
        assert_eq!(
            service.runtime_v2_operation_id(&format!("/v2/instances/instance-1/operations/{id}")),
            Some(id)
        );
    }
    Ok(())
}

#[test]
fn state_route_returns_typed_request_and_explicit_unavailable_fallback() -> Result<(), String> {
    let mut service = test_service()?;
    let request = authenticated_request("/v2/instances/instance-1/state");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["status"], "unavailable");
    assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
    assert_eq!(value["reason"], "downstream_unavailable_before_write");
    assert_eq!(value["request"]["kind"], "state_request");
    assert_eq!(value["request"]["instance_id"], "instance-1");
    assert_eq!(value["request"]["correlation_id"], "corr-state");
    Ok(())
}

#[test]
fn v2_gets_are_not_arbitrary_proxy_routes() -> Result<(), String> {
    let mut service = test_service()?;
    for path in [
        "/v2/instances/instance-1/state/extra",
        "/v2/instances/instance-1/not-a-proxy",
    ] {
        let (status, _) = service.handle_request(&authenticated_request(path));
        assert_eq!(status, 404, "unexpected route match for {path}");
    }
    Ok(())
}

#[test]
fn state_route_accepts_the_typed_mcp_request_body() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v2/instances/instance-1/state");
    request.body = serde_json::to_vec(&RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        0,
    ))
    .map_err(|error| error.to_string())?;
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["reason"], "downstream_unavailable_before_write");
    Ok(())
}

#[test]
fn semantic_adapter_rejects_bounded_gameplay_receipt_routes() -> Result<(), String> {
    let mut service = test_service()?;
    for (method, suffix) in [
        ("GET", "operations/op-1"),
        ("POST", "play-card"),
        ("GET", "state/extra"),
    ] {
        let mut request = authenticated_request(&format!("/v3/instances/instance-1/{suffix}"));
        request.method = method.to_owned();
        request
            .headers
            .insert("content-type".to_owned(), "application/json".to_owned());
        assert_eq!(service.handle_request(&request).0, 404);
    }
    Ok(())
}

#[test]
fn expert_reconcile_keeps_the_native_suffix_out_of_the_get_body() -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let server = std::thread::spawn(move || -> Result<String, String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err(String::from("downstream closed before request headers"));
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let mut response: Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
        )))
        .map_err(|error| error.to_string())?;
        response["correlation_id"] = Value::String(String::from("corr-state"));
        response["status"] = Value::String(String::from("unknown"));
        response["action"] = Value::Null;
        response["observation"] = Value::Null;
        response["transition"] = Value::Null;
        response["error_code"] = Value::String(String::from("sts2.game-mod/outcome_unknown"));
        let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        write!(
            stream,
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .map_err(|error| error.to_string())?;
        stream.write_all(&body).map_err(|error| error.to_string())?;
        String::from_utf8(request).map_err(|error| error.to_string())
    });

    let mut service = test_service()?;
    service.config.mod_address = address;
    let request = authenticated_request("/v4/instances/instance-1/expert-actions/potion-op-1");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let response: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(response["status"], "unknown");
    let request = server
        .join()
        .map_err(|_| String::from("downstream server panicked"))??;
    assert!(request.starts_with("GET /api/v4/runtime/expert-actions/potion-op-1 HTTP/1.1\r\n"));
    assert!(request.contains("Content-Length: 0\r\n"));
    assert!(!request.contains("potion-op-1\r\n\r\n"));
    Ok(())
}
