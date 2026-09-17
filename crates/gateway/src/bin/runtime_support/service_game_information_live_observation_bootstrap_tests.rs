// SPDX-License-Identifier: MIT

use super::super::super::game_information_forwarder::{
    BoundGameInformationCapabilities, GameInformationCapabilities, GameInformationLimits,
};
use super::super::super::game_information_lookup_binding::{
    BoundLookupBinding, LookupBindingScope,
};
use super::super::test_support::{authenticated_request, serve_http_sequence, test_service};
use super::super::*;
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;

const REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-live-observation-bootstrap-v1/golden/bootstrap-request.json"
));
const RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-live-observation-bootstrap-v1/golden/bootstrap-response.json"
));
const UNAVAILABLE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-live-observation-bootstrap-v1/golden/error-native-unavailable.json"
));

fn prepare(service: &mut RuntimeService) {
    service.config.game_information_live_bootstrap_enabled = true;
    service.config.game_information_run_id = String::from("run-42");
    let authority = service.game_information_authority();
    service.game_information_lookup_binding = Some(BoundLookupBinding {
        authority,
        binding_id: String::from("binding-1"),
        content_manifest_id: String::from("content-1"),
        authority_epoch: 7,
        scope: LookupBindingScope {
            project_id: String::from("proj-1"),
            run_id: String::from("run-42"),
            episode_id: String::from("episode-7"),
            agent_id: String::from("agent-3"),
            authority_epoch: 7,
        },
    });
    service.game_information_live_bootstrap_supported = Some(service.game_information_authority());
}

fn request() -> Result<HttpRequest, String> {
    let mut value: Value = serde_json::from_slice(REQUEST).map_err(|error| error.to_string())?;
    value["limits"]["max_message_bytes"] = json!(131_072);
    let mut request = authenticated_request(
        "/v1/instances/instance-1/game-information/live-observation-bootstrap",
    );
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr-bootstrap-1"),
    );
    request.body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    Ok(request)
}

fn response_with_limit(template: &[u8], correlation: &str) -> Result<Vec<u8>, String> {
    let mut value: Value = serde_json::from_slice(template).map_err(|error| error.to_string())?;
    value["correlation_id"] = json!(correlation);
    if value["kind"] == "bootstrap_response" {
        value["limits"]["max_message_bytes"] = json!(131_072);
    }
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

fn install_capabilities(service: &mut RuntimeService) {
    service.game_information_capabilities = Some(BoundGameInformationCapabilities {
        authority: service.game_information_authority(),
        capabilities: GameInformationCapabilities {
            query_kinds: vec![String::from("list")],
            entity_kinds: vec![String::from("card")],
            projections: vec![String::from("summary")],
            detail_levels: vec![String::from("basic")],
            fields: vec![String::from("name")],
            limits: GameInformationLimits {
                page_items: 20,
                item_bytes: 1024,
                page_bytes: 4096,
                text_bytes: 512,
            },
            max_message_bytes: 8192,
            max_cursor_bytes: 128,
            supports_live: false,
        },
    });
}

#[test]
fn bootstrap_forwards_fixed_path_and_accepts_attested_response() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let response = response_with_limit(RESPONSE, "corr-bootstrap-1")?;
    let worker = serve_http_sequence(listener, vec![(200, response.clone())]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    prepare(&mut service);
    let request = request()?;
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, response);
    let forwarded = worker
        .join()
        .map_err(|_| String::from("bootstrap producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    assert_eq!(
        forwarded[0].path,
        "/api/v1/game-information/live-observation-bootstrap"
    );
    assert_eq!(forwarded[0].method, "POST");
    assert_eq!(forwarded[0].body, request.body);
    assert_eq!(
        forwarded[0]
            .headers
            .get("x-sts2-lease-epoch")
            .map(String::as_str),
        Some("1")
    );
    Ok(())
}

#[test]
fn native_unavailable_is_relayed_without_fabricating_observation() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let response = response_with_limit(UNAVAILABLE, "corr-bootstrap-1")?;
    let worker = serve_http_sequence(listener, vec![(503, response.clone())]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    prepare(&mut service);
    let (status, body) = service.handle_request(&request()?);
    assert_eq!(status, 503);
    assert_eq!(body, response);
    assert!(service.game_information_live_bootstrap_supported.is_some());
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
fn foreign_scope_is_rejected_before_producer_io() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut service = test_service()?;
    service.config.mod_address = address;
    prepare(&mut service);
    let mut request = request()?;
    let mut value: Value =
        serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
    value["scope"]["instance_id"] = json!("foreign-instance");
    request.body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let (status, _) = service.handle_request(&request);
    assert_eq!(status, 409);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn stale_generation_response_is_rejected_after_forwarding() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut response: Value =
        serde_json::from_slice(RESPONSE).map_err(|error| error.to_string())?;
    response["correlation_id"] = json!("corr-bootstrap-1");
    response["limits"]["max_message_bytes"] = json!(131_072);
    response["visible_entities"][0]["snapshot_ref"]["state_generation"] = json!(41);
    let worker = serve_http_sequence(
        listener,
        vec![(
            200,
            serde_json::to_vec(&response).map_err(|error| error.to_string())?,
        )],
    );
    let mut service = test_service()?;
    service.config.mod_address = address;
    prepare(&mut service);
    assert_eq!(service.handle_request(&request()?).0, 502);
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
fn unsupported_handler_withdraws_offer_for_current_authority() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(listener, vec![(404, b"{}".to_vec())]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    prepare(&mut service);
    install_capabilities(&mut service);
    assert_eq!(service.handle_request(&request()?).0, 502);
    assert!(service.game_information_live_bootstrap_transport_failed);

    let mut discovery = authenticated_request("/v1/instances/instance-1/negotiated-capabilities");
    discovery.headers.insert(
        String::from("x-sts2-capabilities-version"),
        String::from("sts2-gateway-negotiated-capabilities-v2"),
    );
    let (status, body) = service.handle_request(&discovery);
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(!value["offers"].as_array().is_some_and(|offers| {
        offers
            .iter()
            .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
    }));
    assert_eq!(
        worker
            .join()
            .map_err(|_| String::from("producer panicked"))??
            .len(),
        1
    );
    Ok(())
}
