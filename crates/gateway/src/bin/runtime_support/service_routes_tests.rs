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
    service.runtime_v3 = RuntimeV3GameplayProxy::new(
        &address,
        "mod-token",
        "instance-1",
        "harness",
        "session-1",
        "lease-1",
        1,
        8,
    );
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
        let mut v3: Value = serde_json::from_slice(&v3_action_body()).map_err(|e| e.to_string())?;
        v3["operation_id"] = Value::String(id.to_owned());
        for (version, body) in [
            ("v2", serde_json::to_vec(&v2).map_err(|e| e.to_string())?),
            ("v3", serde_json::to_vec(&v3).map_err(|e| e.to_string())?),
        ] {
            let mut request =
                authenticated_request(&format!("/{version}/instances/instance-1/action"));
            request.method = String::from("POST");
            request.headers.insert(
                String::from("content-type"),
                String::from("application/json"),
            );
            request.body = body;
            let (status, bytes) = service.handle_request(&request);
            assert_eq!(status, 400);
            let response: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            assert_eq!(
                response["error_code"],
                format!("runtime_{version}_operation_invalid")
            );
        }
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
        assert_eq!(
            service.runtime_v3_operation_id(&format!("/v3/instances/instance-1/operations/{id}")),
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
fn v3_state_route_is_authenticated_and_fixed_to_the_gameplay_profile() -> Result<(), String> {
    let mut service = test_service()?;
    let request = authenticated_request("/v3/instances/instance-1/state");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "runtime_v3_downstream_unavailable");

    let (status, _) = service.handle_request(&authenticated_request(
        "/v3/instances/instance-1/state/extra",
    ));
    assert_eq!(status, 404);
    Ok(())
}

#[test]
fn v3_action_validates_the_new_profile_before_forwarding() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v3/instances/instance-1/action");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = v3_action_body();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "runtime_v3_downstream_unavailable");

    request.body = b"{}".to_vec();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 400);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        "runtime_v3_gameplay_unknown_or_missing_field"
    );
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
