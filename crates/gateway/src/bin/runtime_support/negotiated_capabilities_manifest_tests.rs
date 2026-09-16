// SPDX-License-Identifier: MIT

use super::super::test_support::test_service;
use super::tests::{LOOKUP_DISCOVERY_RESPONSE, install_capabilities, request};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::TcpListener;

fn lookup_discovery_request() -> Result<super::super::super::http::HttpRequest, String> {
    let mut request = super::super::test_support::authenticated_request(
        "/v1/instances/instance-1/game-information/lookup-binding",
    );
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

fn discovery_with_different_manifest() -> Result<Vec<u8>, String> {
    let mut response: Value =
        serde_json::from_slice(LOOKUP_DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
    response["binding"]["content_manifest_id"] = json!("content-foreign");
    let binding = response["binding"].as_object().ok_or("binding missing")?;
    let scope = binding
        .get("scope")
        .and_then(Value::as_object)
        .ok_or("binding scope missing")?;
    let mut identity = BTreeMap::<&str, Value>::new();
    for (name, value) in [
        (
            "agent_id",
            scope.get("agent_id").cloned().ok_or("agent id missing")?,
        ),
        ("authority_epoch", json!(7)),
        (
            "content_manifest_id",
            binding
                .get("content_manifest_id")
                .cloned()
                .ok_or("manifest missing")?,
        ),
        (
            "episode_id",
            scope
                .get("episode_id")
                .cloned()
                .ok_or("episode id missing")?,
        ),
        (
            "game_profile",
            binding
                .get("game_profile")
                .cloned()
                .ok_or("game profile missing")?,
        ),
        (
            "locale",
            binding.get("locale").cloned().ok_or("locale missing")?,
        ),
        (
            "project_id",
            scope
                .get("project_id")
                .cloned()
                .ok_or("project id missing")?,
        ),
        (
            "run_id",
            scope.get("run_id").cloned().ok_or("run id missing")?,
        ),
    ] {
        identity.insert(name, value);
    }
    response["binding"]["binding_id"] = json!(sts2_gateway::sha256_hex(
        &serde_json::to_vec(&identity).map_err(|error| error.to_string())?
    ));
    serde_json::to_vec(&response).map_err(|error| error.to_string())
}

#[test]
fn canonical_discovery_for_a_different_manifest_is_rejected_and_never_offered() -> Result<(), String>
{
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = super::super::test_support::serve_http_sequence(
        listener,
        vec![(200, discovery_with_different_manifest()?)],
    );
    let mut service = test_service()?;
    service.config.mod_address = address;
    install_capabilities(&mut service);

    let (status, body) = service.handle_request(&lookup_discovery_request()?);
    assert_eq!(status, 502);
    assert_eq!(
        body,
        br#"{"error_code":"game_information_lookup_binding_response_invalid"}"#
    );
    assert!(service.game_information_lookup_binding.is_none());

    let (status, body) = service.handle_request(&request());
    assert_eq!(status, 200);
    let snapshot: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(snapshot["lookup_binding_witness"].is_null());
    assert!(
        snapshot["offers"]
            .as_array()
            .is_some_and(|offers| offers.iter().all(|offer| !offer["operation"]
                .as_str()
                .is_some_and(
                    |operation| operation.starts_with("game_information.lookup_binding.")
                )))
    );

    let forwarded = producer
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    Ok(())
}
