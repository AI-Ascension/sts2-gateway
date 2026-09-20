// SPDX-License-Identifier: MIT

use super::super::super::game_information::GameInformationRoute;
use super::super::super::game_information_content_manifest as manifest;
use super::super::test_support::{authenticated_request, serve_http_sequence, test_service};
use super::super::*;
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;

const MANIFEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-content-manifest-v1/golden/canonical-manifest-response.json"
));
const DENIED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-content-manifest-v1/golden/access-denied-error-response.json"
));
const SCHEMA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-content-manifest-v1/schema.json"
));
const INVENTORY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const CORRELATION: &str = "corr-content-manifest-1";
const PATH: &str = "/v1/instances/instance-1/game-information/content-manifest";
const DOWNSTREAM: &str = "/api/v1/game-information/content-manifest";

fn request() -> HttpRequest {
    let mut request = authenticated_request(PATH);
    request.headers.insert(
        String::from(manifest::CORRELATION_HEADER),
        String::from(CORRELATION),
    );
    request
}

fn envelope(template: &[u8], correlation: &str) -> Result<Vec<u8>, String> {
    let mut value: Value = serde_json::from_slice(template).map_err(|error| error.to_string())?;
    value["correlation_id"] = json!(correlation);
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

type Producer = std::thread::JoinHandle<Result<Vec<HttpRequest>, String>>;

fn serve(responses: Vec<(u16, Vec<u8>)>) -> Result<(String, Producer), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    Ok((address, serve_http_sequence(listener, responses)))
}

/// A listener that must observe no connection, so a rejected request proves it never forwarded.
fn trap() -> Result<(String, TcpListener), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    Ok((address, listener))
}

fn assert_no_forward(listener: &TcpListener) {
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
}

#[test]
fn content_manifest_route_is_fixed_and_is_not_a_query_operation() {
    assert_eq!(
        GameInformationRoute::parse("GET", PATH, "instance-1"),
        Some(GameInformationRoute::ContentManifest)
    );
    assert_eq!(
        GameInformationRoute::ContentManifest.downstream_path(),
        DOWNSTREAM
    );
    assert!(!GameInformationRoute::ContentManifest.is_query());
    assert_eq!(GameInformationRoute::ContentManifest.query_kind(), None);
    assert_eq!(
        GameInformationRoute::parse("POST", PATH, "instance-1"),
        None,
        "the manifest read is a bodyless GET only"
    );
    assert_eq!(
        GameInformationRoute::parse(
            "GET",
            "/v1/instances/instance-1/game-information/content-manifests",
            "instance-1"
        ),
        None
    );
}

#[test]
fn manifest_is_forwarded_to_the_fixed_path_and_relayed_verbatim() -> Result<(), String> {
    let body = envelope(MANIFEST, CORRELATION)?;
    let (address, worker) = serve(vec![(200, body.clone())])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let (status, response) = service.handle_request(&request());
    assert_eq!(status, 200);
    assert_eq!(response, body);
    let forwarded = worker
        .join()
        .map_err(|_| String::from("producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].method, "GET");
    assert_eq!(forwarded[0].path, DOWNSTREAM);
    assert!(forwarded[0].body.is_empty());
    let header = |name: &str| forwarded[0].headers.get(name).map(String::as_str);
    assert_eq!(header(manifest::CORRELATION_HEADER), Some(CORRELATION));
    assert_eq!(header("x-sts2-lease-id"), Some("lease-1"));
    assert_eq!(header("x-sts2-lease-epoch"), Some("1"));
    assert_eq!(header("x-sts2-instance-id"), Some("instance-1"));
    assert_eq!(header("x-sts2-locale"), Some("en-US"));
    Ok(())
}

#[test]
fn typed_producer_error_is_relayed_without_fabricating_a_manifest() -> Result<(), String> {
    let body = envelope(DENIED, CORRELATION)?;
    let (address, worker) = serve(vec![(403, body.clone())])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let (status, response) = service.handle_request(&request());
    assert_eq!(status, 403);
    assert_eq!(response, body);
    let value: Value = serde_json::from_slice(&response).map_err(|error| error.to_string())?;
    assert_eq!(value["kind"], "error_response");
    assert!(value["manifest"].is_null());
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1
    );
    Ok(())
}

/// A manifest the producer declares larger than the admitted bound is refused from the
/// declaration, so no producer byte can be read as the start of a shorter, plausible catalog.
#[test]
fn declared_oversize_is_refused_with_the_protocol_limit_arm() -> Result<(), String> {
    let (address, worker) = serve(vec![(200, vec![b'x'; 200_000])])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let (status, response) = service.handle_request(&request());
    assert_eq!(status, 413);
    assert_eq!(response, manifest::refusal_envelope(CORRELATION));
    assert!(response.len() <= manifest::MAX_MANIFEST_BYTES);
    let value: Value = serde_json::from_slice(&response).map_err(|error| error.to_string())?;
    assert_eq!(value["kind"], "error_response");
    assert_eq!(value["correlation_id"], CORRELATION);
    assert_eq!(value["error"]["code"], "result_limit_exceeded");
    assert_eq!(value["error"]["reason"], "serialized_payload_too_large");
    assert!(value["manifest"].is_null());
    // The producer thread needs no assertion: it may observe the close of a connection the gateway
    // refused before reading the declared body, which is the point of the arm.
    let _ = worker.join();
    Ok(())
}

#[test]
fn pinned_revision_fence_admits_the_bound_revision() -> Result<(), String> {
    let body = envelope(MANIFEST, CORRELATION)?;
    let (address, worker) = serve(vec![(200, body.clone())])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.game_information_content_manifest_id = String::from(INVENTORY);
    let (status, response) = service.handle_request(&request());
    assert_eq!(status, 200);
    assert_eq!(response, body);
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1
    );
    Ok(())
}

#[test]
fn pinned_revision_fence_refuses_a_foreign_inventory_revision() -> Result<(), String> {
    let body = envelope(MANIFEST, CORRELATION)?;
    let (address, worker) = serve(vec![(200, body)])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.game_information_content_manifest_id = "d".repeat(64);
    let (status, _) = service.handle_request(&request());
    assert_eq!(status, 409);
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1,
        "the revision fence is decided after the exchange, not instead of it"
    );
    Ok(())
}

#[test]
fn foreign_correlation_is_rejected_after_forwarding() -> Result<(), String> {
    let body = envelope(MANIFEST, "corr-foreign")?;
    let (address, worker) = serve(vec![(200, body)])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    assert_eq!(service.handle_request(&request()).0, 502);
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1
    );
    Ok(())
}

#[test]
fn drifted_schema_digest_is_refused() -> Result<(), String> {
    let mut value: Value = serde_json::from_slice(MANIFEST).map_err(|error| error.to_string())?;
    value["correlation_id"] = json!(CORRELATION);
    value["schema_digest"] = json!("0".repeat(64));
    let body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let (address, worker) = serve(vec![(200, body)])?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    assert_eq!(service.handle_request(&request()).0, 502);
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1
    );
    Ok(())
}

#[test]
fn unrelated_operation_under_the_same_prefix_is_not_a_route() {
    assert_eq!(
        GameInformationRoute::parse(
            "GET",
            "/v1/instances/instance-1/game-information/content-manifest/extra",
            "instance-1"
        ),
        None
    );
}

#[test]
fn missing_correlation_is_rejected_before_producer_io() -> Result<(), String> {
    let (address, trap) = trap()?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut request = request();
    request.headers.remove(manifest::CORRELATION_HEADER);
    let (status, _) = service.handle_request(&request);
    assert_eq!(status, 400);
    assert_no_forward(&trap);
    Ok(())
}

#[test]
fn unsafe_correlation_is_rejected_before_producer_io() -> Result<(), String> {
    let (address, trap) = trap()?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut request = request();
    request.headers.insert(
        String::from(manifest::CORRELATION_HEADER),
        String::from("a b"),
    );
    let (status, _) = service.handle_request(&request);
    assert_eq!(status, 400);
    assert_no_forward(&trap);
    Ok(())
}

#[test]
fn body_bearing_get_is_rejected_before_producer_io() -> Result<(), String> {
    let (address, trap) = trap()?;
    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut request = request();
    request.body = b"{}".to_vec();
    let (status, _) = service.handle_request(&request);
    assert_eq!(status, 400);
    assert_no_forward(&trap);
    Ok(())
}

#[test]
fn unreachable_producer_is_reported_as_unavailable() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    drop(listener);
    let mut service = test_service()?;
    service.config.mod_address = address;
    assert_eq!(service.handle_request(&request()).0, 503);
    Ok(())
}

/// The refusal arm must be the pinned schema's own token pair, and the admitted bound must stay
/// smaller than the 16 MiB message ceiling the profile permits.
#[test]
fn refusal_arm_and_admitted_bound_match_the_pinned_artifact() -> Result<(), String> {
    let envelope: Value = serde_json::from_slice(&manifest::refusal_envelope(CORRELATION))
        .map_err(|error| error.to_string())?;
    let schema: Value = serde_json::from_slice(SCHEMA).map_err(|error| error.to_string())?;
    let code = &schema["$defs"]["error_payload"]["properties"]["code"]["enum"];
    assert!(
        code.as_array()
            .is_some_and(|codes| codes.contains(&envelope["error"]["code"])),
        "the refusal code must be the pinned schema's own closed token"
    );
    let reason = &schema["$defs"]["reason"]["enum"];
    assert!(
        reason
            .as_array()
            .is_some_and(|reasons| reasons.contains(&envelope["error"]["reason"])),
        "the refusal reason must be the pinned schema's own closed token"
    );
    let mut headers = std::collections::BTreeMap::new();
    headers.insert(
        String::from(manifest::CORRELATION_HEADER),
        String::from(CORRELATION),
    );
    manifest::validate_response(
        &manifest::refusal_envelope(CORRELATION),
        &headers,
        None,
        413,
    )
    .map_err(|error| format!("{error:?}"))?;
    const { assert!(manifest::MAX_MANIFEST_BYTES == 128 * 1024) };
    const { assert!(manifest::MAX_MANIFEST_BYTES < 16777216) };
    Ok(())
}
