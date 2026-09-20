// SPDX-License-Identifier: MIT

use super::test_support::{
    authenticated_request, game_information_capabilities_request, serve_http_sequence, test_service,
};
use super::*;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;

const STATIC_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-request.json"
));
const STATIC_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-response.json"
));
const LIVE_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/live-detail-request.json"
));
const LIVE_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/live-detail-response.json"
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

fn capabilities_request() -> HttpRequest {
    game_information_capabilities_request()
}

fn replace(body: &[u8], from: &str, to: &str) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    if text.matches(from).count() != 1 {
        return Err(format!("expected one occurrence of {from:?}"));
    }
    Ok(text.replacen(from, to, 1).into_bytes())
}

fn service_with_address(address: String) -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    service.config.mod_address = address;
    Ok(service)
}

fn serve_once(
    listener: TcpListener,
    response: Vec<u8>,
) -> thread::JoinHandle<Result<HttpRequest, String>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let request = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
        write_response(&mut stream, 200, &response).map_err(|error| error.to_string())?;
        Ok(request)
    })
}

#[test]
fn static_list_and_live_detail_use_the_exact_fixed_producer_paths() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let first = serve_http_sequence(
        listener,
        vec![
            (200, CAPABILITIES_RESPONSE.to_vec()),
            (200, STATIC_RESPONSE.to_vec()),
        ],
    );
    let mut service = service_with_address(address)?;
    let (status, _) = service.handle_request(&capabilities_request());
    assert_eq!(status, 200);
    let request = query_request(
        "/v1/instances/instance-1/game-information/query",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, STATIC_RESPONSE);
    let forwarded = first
        .join()
        .map_err(|_| String::from("static producer panicked"))??
        .into_iter()
        .nth(1)
        .ok_or_else(|| String::from("missing static request"))?;
    assert_eq!(forwarded.method, "POST");
    assert_eq!(forwarded.path, "/api/v1/game-information/list");
    assert_eq!(forwarded.body, STATIC_REQUEST);
    assert_eq!(
        forwarded.headers.get("authorization").map(String::as_str),
        Some("Bearer mod-token")
    );
    for (name, value) in [
        ("x-sts2-instance-id", "instance-1"),
        ("x-sts2-caller-id", "harness"),
        ("x-sts2-session-id", "session-1"),
        ("x-sts2-lease-id", "lease-1"),
        ("x-sts2-lease-epoch", "1"),
        ("x-sts2-correlation-id", "corr-static-page-1"),
    ] {
        assert_eq!(forwarded.headers.get(name).map(String::as_str), Some(value));
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let second = serve_http_sequence(
        listener,
        vec![
            (200, CAPABILITIES_RESPONSE.to_vec()),
            (200, LIVE_RESPONSE.to_vec()),
        ],
    );
    service.config.mod_address = address;
    service.config.lease_epoch = 7;
    let mut capabilities = capabilities_request();
    capabilities
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("7"));
    let (status, _) = service.handle_request(&capabilities);
    assert_eq!(status, 200);
    let mut request = query_request(
        "/v1/instances/instance-1/game-information/detail",
        LIVE_REQUEST,
    );
    request
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("7"));
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, LIVE_RESPONSE);
    let forwarded = second
        .join()
        .map_err(|_| String::from("live producer panicked"))??
        .into_iter()
        .nth(1)
        .ok_or_else(|| String::from("missing live request"))?;
    assert_eq!(forwarded.method, "POST");
    assert_eq!(forwarded.path, "/api/v1/game-information/detail");
    assert_eq!(forwarded.body, LIVE_REQUEST);
    assert_eq!(
        forwarded
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str),
        Some("corr-live-detail")
    );
    Ok(())
}

#[test]
fn capabilities_are_a_bodyless_read_and_unknown_operations_never_forward() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_once(listener, CAPABILITIES_RESPONSE.to_vec());
    let mut service = service_with_address(address)?;
    let mut request = capabilities_request();
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr-capabilities"),
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, CAPABILITIES_RESPONSE);
    let forwarded = worker
        .join()
        .map_err(|_| String::from("capability producer panicked"))??;
    assert_eq!(forwarded.method, "GET");
    assert_eq!(forwarded.path, "/api/v1/game-information/capabilities");
    assert!(forwarded.body.is_empty());

    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let unknown =
        authenticated_request("/v1/instances/instance-1/game-information/not-allowlisted");
    assert_eq!(service.handle_request(&unknown).0, 404);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn stale_or_wrong_scope_is_rejected_before_a_producer_connection() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut service = service_with_address(address)?;
    for (header, value) in [
        ("x-sts2-instance-id", "other"),
        ("x-sts2-caller-id", "other-caller"),
        ("x-sts2-session-id", "other-session"),
        ("x-sts2-lease-id", "other-lease"),
        ("x-sts2-lease-epoch", "2"),
    ] {
        let mut request = query_request(
            "/v1/instances/instance-1/game-information/list",
            STATIC_REQUEST,
        );
        request.headers.insert(header.to_owned(), value.to_owned());
        assert_eq!(service.handle_request(&request).0, 409, "{header}");
    }
    let wrong_scope = replace(
        STATIC_REQUEST,
        "\"visibility_scope\":\"public\"",
        "\"visibility_scope\":\"owner\"",
    )?;
    let request = query_request(
        "/v1/instances/instance-1/game-information/list",
        &wrong_scope,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}

#[path = "service_game_information_live_observation_bootstrap_tests.rs"]
mod live_observation_bootstrap_tests;

#[path = "service_game_information_content_manifest_tests.rs"]
mod content_manifest_tests;
