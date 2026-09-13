// SPDX-License-Identifier: MIT

use super::test_support::{
    authenticated_request, game_information_capabilities_request, serve_http_sequence, test_service,
};
use super::*;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpListener;

const STATIC_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-request.json"
));
const CAPABILITIES_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/capabilities-response.json"
));

fn query_request(path: &str, body: &[u8]) -> HttpRequest {
    let mut request = authenticated_request(path);
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = body.to_vec();
    if let Ok(value) = serde_json::from_slice::<Value>(body)
        && let Some(correlation) = value.get("correlation_id").and_then(Value::as_str)
    {
        request.headers.insert(
            String::from("x-sts2-correlation-id"),
            correlation.to_owned(),
        );
    }
    request
}

fn capabilities_with(mutate: impl FnOnce(&mut Value)) -> Result<Vec<u8>, String> {
    let mut value: Value =
        serde_json::from_slice(CAPABILITIES_RESPONSE).map_err(|error| error.to_string())?;
    mutate(&mut value);
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

fn service_with_address(address: String) -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    service.config.mod_address = address;
    Ok(service)
}

#[test]
fn game_information_queries_require_negotiated_capabilities() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut service = service_with_address(address)?;
    let request = query_request(
        "/v1/instances/instance-1/game-information/query",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        "game_information_capabilities_unavailable"
    );
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn rejected_capability_readiness_cannot_poison_query_admission() -> Result<(), String> {
    let capabilities = capabilities_with(|value| {
        value["capabilities"]["limits"]["page_items"] = Value::from(33_u64);
    })?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(listener, vec![(200, capabilities)]);
    let mut service = service_with_address(address)?;
    let (status, _) = service.handle_request(&game_information_capabilities_request());
    assert_eq!(status, 502);
    worker
        .join()
        .map_err(|_| String::from("capability producer panicked"))??;

    let request = query_request(
        "/v1/instances/instance-1/game-information/query",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        "game_information_capabilities_unavailable"
    );
    Ok(())
}

#[test]
fn negotiated_limits_and_operations_reject_before_a_query_connection() -> Result<(), String> {
    let limited_capabilities = capabilities_with(|value| {
        value["capabilities"]["limits"]["page_items"] = Value::from(1_u64);
    })?;
    let operation_capabilities = capabilities_with(|value| {
        value["capabilities"]["query_kinds"] = serde_json::json!(["detail"]);
    })?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(
        listener,
        vec![(200, limited_capabilities), (200, operation_capabilities)],
    );
    let mut service = service_with_address(address)?;
    let (status, _) = service.handle_request(&game_information_capabilities_request());
    assert_eq!(status, 200);

    let oversized = query_request(
        "/v1/instances/instance-1/game-information/query",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&oversized);
    assert_eq!(status, 413);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "game_information_limits_rejected");

    let (status, _) = service.handle_request(&game_information_capabilities_request());
    assert_eq!(status, 200);
    let unsupported = query_request(
        "/v1/instances/instance-1/game-information/query",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&unsupported);
    assert_eq!(status, 503);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        "game_information_capabilities_unavailable"
    );
    worker
        .join()
        .map_err(|_| String::from("capability producer panicked"))??;
    Ok(())
}
