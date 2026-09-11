// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::super::strict_json;
use super::test_support::{authenticated_request, test_service};
use super::*;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

pub(super) const OBSERVATION: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/observation-response.json"
));
const ACTION_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-settled-request.json"
));
const ACTION_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-settled-response.json"
));
pub(super) const LEGAL_CATALOG_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/legal-catalog-request.json"
));
pub(super) const LEGAL_CATALOG_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/legal-catalog-response.json"
));
pub(super) const UNKNOWN_ACTION_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-unknown-request.json"
));
pub(super) const UNKNOWN_ACTION_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-unknown-response.json"
));
pub(super) const RECOVERED_ACTION_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-recovered-request.json"
));
pub(super) const RECOVERED_ACTION_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-recovered-response.json"
));
const CANONICAL_LOCAL_PEER_ID: &str = "peer:host1";

pub(super) fn fixture(bytes: &[u8]) -> Value {
    strict_json::parse(bytes).expect("fixture must be strict JSON")
}

pub(super) fn configure_for(service: &mut RuntimeService, value: &Value) -> HttpRequest {
    let instance = value["instance_id"].as_str().unwrap().to_owned();
    let session = value["session_id"].as_str().unwrap().to_owned();
    let lease = value["lease_id"].as_str().unwrap().to_owned();
    let correlation = value["correlation_id"].as_str().unwrap().to_owned();
    let epoch = value["lease_epoch"].as_u64().unwrap();
    service.config.instance_id = instance.clone();
    service.config.session_id = session.clone();
    service.config.lease_id = lease.clone();
    service.config.lease_epoch = epoch;
    service.coop_native_peer_binding = Some(CoopNativePeerBinding {
        peer_token: String::from("peer-token-1"),
        peer_id: String::from(CANONICAL_LOCAL_PEER_ID),
        instance_id: instance.clone(),
        session_id: session.clone(),
        lease_id: lease.clone(),
        lease_epoch: epoch,
    });
    let mut request = authenticated_request(&format!(
        "/v1/instances/{instance}/coop/native/observation"
    ));
    request.headers.insert("x-sts2-instance-id".into(), instance);
    request.headers.insert("x-sts2-session-id".into(), session);
    request.headers.insert("x-sts2-lease-id".into(), lease);
    request
        .headers
        .insert("x-sts2-lease-epoch".into(), epoch.to_string());
    request
        .headers
        .insert("x-sts2-correlation-id".into(), correlation);
    request
        .headers
        .insert("x-sts2-peer-token".into(), String::from("peer-token-1"));
    request
}

pub(super) fn serve_once(
    listener: TcpListener,
    expected_path: &'static str,
    response_status: u16,
    response_body: Vec<u8>,
) -> thread::JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let forwarded = read_request(&mut stream).map_err(|error| error.to_string())?;
        if forwarded.path != expected_path {
            return Err(format!(
                "expected fixed downstream path {expected_path}, got {}",
                forwarded.path
            ));
        }
        write_response(&mut stream, response_status, &response_body)
            .map_err(|error| error.to_string())
    })
}

#[test]
fn native_observation_forwards_only_the_fixed_path() -> Result<(), String> {
    let observation = fixture(OBSERVATION);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &observation);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    service.config.mod_address = address.to_string();
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/observation",
        200,
        OBSERVATION.to_vec(),
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, OBSERVATION);
    worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;

    request.path = format!(
        "/v1/instances/{}/coop/native/observation/extra",
        service.config.instance_id
    );
    assert_eq!(service.handle_request(&request).0, 404);
    Ok(())
}

#[test]
fn native_action_preserves_body_identity_and_rejects_invalid_response() -> Result<(), String> {
    let action_request = fixture(ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action_request);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());
    request.body = ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    service.config.mod_address = address.to_string();
    let response_body = ACTION_RESPONSE.to_vec();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let forwarded = read_request(&mut stream).map_err(|error| error.to_string())?;
        if forwarded.method != "POST"
            || forwarded.path != "/api/v1/coop/native/action"
            || forwarded.body != ACTION_REQUEST
            || forwarded.headers.get("authorization").map(String::as_str)
                != Some("Bearer mod-token")
            || forwarded.headers.contains_key("x-sts2-peer-token")
        {
            return Err(String::from("native action forwarding changed at gateway"));
        }
        write_response(&mut stream, 200, &response_body).map_err(|error| error.to_string())
    });
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, ACTION_RESPONSE);
    worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;

    let mut bad_request = request;
    bad_request.body = b"{}".to_vec();
    assert_eq!(service.handle_request(&bad_request).0, 400);
    Ok(())
}

#[test]
fn native_route_requires_its_scope_before_lease_or_downstream() -> Result<(), String> {
    for (suffix, method, scope, expected) in [
        ("observation", "GET", "read", 503),
        ("legal-catalog", "POST", "read", 400),
        ("action", "POST", "mutate", 400),
        ("vote", "POST", "mutate", 400),
        ("rejoin", "POST", "control", 400),
        ("recover", "POST", "control", 400),
    ] {
        let mut service = test_service()?;
        service.config.auth_policy = AuthPolicy::test_with_previous(
            "gateway-token",
            None,
            None,
            scope,
        )?;
        let mut request = authenticated_request(&format!(
            "/v1/instances/instance-1/coop/native/{suffix}"
        ));
        request.method = method.to_owned();
        if method == "POST" {
            request.headers.insert("content-type".into(), "application/json".into());
            request.body = b"{}".to_vec();
        }
        assert_eq!(service.handle_request(&request).0, expected, "{suffix} {scope}");
    }
    Ok(())
}

#[test]
fn native_route_binding_rejects_peer_substitution_and_stale_lease() -> Result<(), String> {
    let action = fixture(ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());

    request.headers.remove("x-sts2-peer-token");
    request.body = ACTION_REQUEST.to_vec();
    assert_eq!(service.handle_request(&request).0, 401);
    request
        .headers
        .insert("x-sts2-peer-token".into(), String::from("peer-token-wrong"));
    assert_eq!(service.handle_request(&request).0, 401);
    request
        .headers
        .insert("x-sts2-peer-token".into(), String::from("peer-token-1"));

    let mut substituted = action.clone();
    substituted["actor_peer"] = Value::String(String::from("peer:substituted"));
    request.body = serde_json::to_vec(&substituted).map_err(|error| error.to_string())?;
    assert_eq!(service.handle_request(&request).0, 409);

    request.body = ACTION_REQUEST.to_vec();
    request
        .headers
        .insert("x-sts2-lease-epoch".into(), String::from("6"));
    assert_eq!(service.handle_request(&request).0, 409);
    Ok(())
}

#[test]
fn native_pending_duplicate_and_recovery_are_bound_to_original_operation() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/action",
        200,
        UNKNOWN_ACTION_RESPONSE.to_vec(),
    );
    assert_eq!(service.handle_request(&request).0, 200);
    worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;

    assert_eq!(service.handle_request(&request).0, 409);

    let recovered = fixture(RECOVERED_ACTION_REQUEST);
    let mut recovery = configure_for(&mut service, &recovered);
    recovery.method = String::from("POST");
    recovery.path = format!(
        "/v1/instances/{}/coop/native/recover",
        service.config.instance_id
    );
    recovery
        .headers
        .insert("content-type".into(), "application/json".into());
    recovery.body = RECOVERED_ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/recover",
        200,
        RECOVERED_ACTION_RESPONSE.to_vec(),
    );
    assert_eq!(service.handle_request(&recovery).0, 200);
    worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;
    assert!(service.coop_native_pending.is_none());
    Ok(())
}

#[test]
fn native_explicit_host_rejection_releases_the_pending_operation() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/action",
        409,
        br#"{"error_code":"native_action_rejected"}"#.to_vec(),
    );
    assert_eq!(service.handle_request(&request).0, 409);
    worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;
    assert!(service.coop_native_pending.is_none());
    Ok(())
}
