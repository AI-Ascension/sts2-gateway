// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::super::auth::AuthPolicy;
use super::super::super::game_information_forwarder::{
    BoundGameInformationCapabilities, GameInformationCapabilities, GameInformationLimits,
};
use super::super::test_support::{authenticated_request, serve_http_sequence, test_service};
use super::{SCHEMA_VERSION, path};
use std::net::TcpListener;

const CAPABILITIES_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/capabilities-response.json"
));
pub(super) const LOOKUP_DISCOVERY_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-lookup-binding-v1/golden/discovery-response.json"
));

pub(super) fn request() -> super::super::super::http::HttpRequest {
    authenticated_request(&path("instance-1"))
}

fn capabilities_response_for(
    request: &super::super::super::http::HttpRequest,
) -> Result<Vec<u8>, String> {
    let correlation = request
        .headers
        .get("x-sts2-correlation-id")
        .ok_or("correlation header missing")?;
    let mut response: Value =
        serde_json::from_slice(CAPABILITIES_RESPONSE).map_err(|error| error.to_string())?;
    response["correlation_id"] = Value::String(correlation.clone());
    serde_json::to_vec(&response).map_err(|error| error.to_string())
}

fn lookup_discovery_request() -> Result<super::super::super::http::HttpRequest, String> {
    let mut request =
        authenticated_request("/v1/instances/instance-1/game-information/lookup-binding");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr-lbr-discovery-1"),
    );
    request.body = serde_json::to_vec(&json!({
        "operation": "discovery",
        "project_id": "proj-1",
        "run_id": "run-42",
        "episode_id": "episode-7",
        "agent_id": "agent-3",
        "authority_epoch": 7,
    }))
    .map_err(|error| error.to_string())?;
    Ok(request)
}

pub(super) fn install_capabilities(service: &mut super::super::RuntimeService) {
    service.game_information_capabilities = Some(BoundGameInformationCapabilities {
        authority: service.game_information_authority(),
        capabilities: GameInformationCapabilities {
            query_kinds: vec![String::from("list"), String::from("unknown")],
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
fn route_projects_validated_producer_evidence_and_actual_grants() -> Result<(), String> {
    let mut service = test_service()?;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read,mutate")?;
    install_capabilities(&mut service);

    let (status, body) = service.handle_request(&request());
    if status != 200 {
        return Err(format!("unexpected status {status}"));
    }
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["schema_version"], SCHEMA_VERSION);
    assert_eq!(value["instance_id"], "instance-1");
    assert_eq!(value["session_id"], "session-1");
    assert_eq!(value["mcp_session_id"], "mcp-session-1");
    assert_eq!(value["lease_id"], "lease-1");
    assert_eq!(value["lease_epoch"], 1);
    assert_eq!(
        value["caller_scopes"],
        serde_json::json!(["read", "mutate"])
    );
    assert_eq!(value["producer"]["profile"], "game-information-query-v1");
    assert!(value["lookup_binding_witness"].is_null());
    assert_eq!(
        value["offers"],
        serde_json::json!([
            {
                "operation":"game_information.capabilities",
                "revision":"game-information-query-v1",
                "required_scope":"read",
                "scope":["read", "mutate"],
                "wire_limits":{
                    "max_request_bytes":0,
                    "max_response_bytes":8192
                },
                "content_limits":{
                    "max_content_bytes":8192,
                    "max_page_items":1
                }
            },
            {
                "operation":"game_information.list",
                "revision":"game-information-query-v1",
                "required_scope":"read",
                "scope":["read", "mutate"],
                "wire_limits":{
                    "max_request_bytes":8192,
                    "max_response_bytes":8192
                },
                "content_limits":{
                    "max_content_bytes":4096,
                    "max_page_items":20
                }
            }
        ])
    );
    Ok(())
}

#[test]
fn route_advertises_only_a_validated_current_lookup_binding_discovery() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(listener, vec![(200, LOOKUP_DISCOVERY_RESPONSE.to_vec())]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    install_capabilities(&mut service);

    assert_eq!(
        service.handle_request(&lookup_discovery_request()?).0,
        200,
        "validated discovery"
    );
    let (status, body) = service.handle_request(&request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["lookup_binding_witness"],
        json!({
            "profile":"game-information-lookup-binding-v1",
            "schema_digest":"f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58",
            "binding_id":"58fea90991138ea6fb635df1f5eadd08973ec63eba456d135578677ffee61cfc",
            "content_manifest_id":"content-1",
            "authority_epoch":7
        })
    );
    let operations = value["offers"]
        .as_array()
        .ok_or("offers missing")?
        .iter()
        .filter_map(|offer| offer["operation"].as_str())
        .collect::<Vec<_>>();
    assert!(operations.contains(&"game_information.lookup_binding.discovery"));
    assert!(operations.contains(&"game_information.lookup_binding.observe"));

    service
        .game_information_lookup_binding
        .as_mut()
        .ok_or("lookup witness missing")?
        .authority
        .lease_id = String::from("foreign-lease");
    let (_, body) = service.handle_request(&request());
    let stale: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(stale["lookup_binding_witness"].is_null());
    assert!(
        stale["offers"]
            .as_array()
            .is_some_and(|offers| offers.iter().all(|offer| {
                !offer["operation"].as_str().is_some_and(|operation| {
                    operation.starts_with("game_information.lookup_binding.")
                })
            }))
    );

    let forwarded = producer
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].path, "/api/v1/game-information/lookup-binding");
    Ok(())
}

#[test]
fn failed_replacement_discovery_clears_the_prior_lookup_witness() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(
        listener,
        vec![
            (200, LOOKUP_DISCOVERY_RESPONSE.to_vec()),
            (200, b"{}".to_vec()),
        ],
    );
    let mut service = test_service()?;
    service.config.mod_address = address;
    install_capabilities(&mut service);

    let discovery = lookup_discovery_request()?;
    assert_eq!(service.handle_request(&discovery).0, 200);
    assert!(service.game_information_lookup_binding.is_some());
    assert_eq!(service.handle_request(&discovery).0, 502);
    assert!(service.game_information_lookup_binding.is_none());

    let (_, body) = service.handle_request(&request());
    let snapshot: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(snapshot["lookup_binding_witness"].is_null());
    assert!(
        snapshot["offers"]
            .as_array()
            .is_some_and(|offers| offers.iter().all(|offer| {
                !offer["operation"].as_str().is_some_and(|operation| {
                    operation.starts_with("game_information.lookup_binding.")
                })
            }))
    );
    let forwarded = producer
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 2);
    Ok(())
}

#[test]
fn route_refreshes_cold_cache_through_validated_producer_discovery() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request = request();
    let producer = serve_http_sequence(listener, vec![(200, capabilities_response_for(&request)?)]);
    let mut service = test_service()?;
    service.config.mod_address = address;

    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["offers"].as_array().map(Vec::len), Some(6));
    assert_eq!(
        value["offers"][0]["operation"],
        "game_information.capabilities"
    );
    let requests = producer
        .join()
        .map_err(|_| String::from("producer fixture panicked"))??;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/v1/game-information/capabilities");
    Ok(())
}

#[test]
fn route_refreshes_stale_capabilities_before_projecting_offers() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request = request();
    let producer = serve_http_sequence(listener, vec![(200, capabilities_response_for(&request)?)]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    install_capabilities(&mut service);
    service
        .game_information_capabilities
        .as_mut()
        .ok_or("missing cache")?
        .authority
        .lease_id = String::from("foreign-lease");

    assert_eq!(service.handle_request(&request).0, 200);
    let requests = producer
        .join()
        .map_err(|_| String::from("producer fixture panicked"))??;
    assert_eq!(requests.len(), 1);
    Ok(())
}

#[test]
fn route_requires_current_lease_and_read_scope_before_producer_discovery() -> Result<(), String> {
    let mut service = test_service()?;
    let mut wrong_lease = request();
    wrong_lease
        .headers
        .insert(String::from("x-sts2-lease-id"), String::from("foreign"));
    assert_eq!(service.handle_request(&wrong_lease).0, 409);

    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "mutate")?;
    assert_eq!(service.handle_request(&request()).0, 403);
    Ok(())
}

#[test]
fn route_rejects_a_body_before_reading_capability_state() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = request();
    request.body = b"{}".to_vec();
    assert_eq!(service.handle_request(&request).0, 400);
    Ok(())
}

#[test]
fn route_rejects_an_untyped_producer_failure_on_cold_discovery() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(
        listener,
        vec![(503, br#"{"code":"source_unavailable"}"#.to_vec())],
    );
    let mut service = test_service()?;
    service.config.mod_address = address;

    assert_eq!(service.handle_request(&request()).0, 502);
    let requests = producer
        .join()
        .map_err(|_| String::from("producer fixture panicked"))??;
    assert_eq!(requests.len(), 1);
    Ok(())
}
