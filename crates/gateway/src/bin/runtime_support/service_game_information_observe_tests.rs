// SPDX-License-Identifier: MIT

use super::super::test_support::serve_http_sequence;
use super::{DISCOVERY_RESPONSE, OBSERVATION_RESPONSE, lookup_request, service_with_address};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::TcpListener;

fn canonical_observation_for_different_manifest() -> Result<Vec<u8>, String> {
    let mut response: Value =
        serde_json::from_slice(OBSERVATION_RESPONSE).map_err(|error| error.to_string())?;
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
    let binding_id = sts2_gateway::sha256_hex(
        &serde_json::to_vec(&identity).map_err(|error| error.to_string())?,
    );
    response["binding"]["binding_id"] = json!(binding_id);
    response["observation"]["binding_id"] = json!(binding_id);
    serde_json::to_vec(&response).map_err(|error| error.to_string())
}

#[test]
fn observation_requires_a_current_matching_discovery_and_clears_a_foreign_manifest()
-> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(
        listener,
        vec![
            (200, DISCOVERY_RESPONSE.to_vec()),
            (200, canonical_observation_for_different_manifest()?),
        ],
    );
    let mut service = service_with_address(address)?;

    assert_eq!(
        service
            .handle_request(&lookup_request("discovery", "corr-lbr-discovery-1")?)
            .0,
        200
    );
    assert!(service.game_information_lookup_binding.is_some());

    let (status, body) =
        service.handle_request(&lookup_request("observe", "corr-lbr-observation-1")?);
    assert_eq!(status, 502);
    assert_eq!(
        body,
        br#"{"error_code":"game_information_lookup_binding_response_invalid"}"#
    );
    assert!(service.game_information_lookup_binding.is_none());

    let forwarded = producer
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 2);
    Ok(())
}

#[test]
fn observation_without_discovery_is_refused_by_the_pinned_contract() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut service = service_with_address(address)?;

    assert_eq!(
        service
            .handle_request(&lookup_request("observe", "corr-lbr-observation-1")?)
            .0,
        409
    );
    assert!(service.game_information_lookup_binding.is_none());
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn stale_or_different_discovery_scope_rejects_observation_before_producer_io() -> Result<(), String>
{
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(listener, vec![(200, DISCOVERY_RESPONSE.to_vec())]);
    let mut service = service_with_address(address)?;
    assert_eq!(
        service
            .handle_request(&lookup_request("discovery", "corr-lbr-discovery-1")?)
            .0,
        200
    );
    let forwarded = producer
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 1, "only discovery reaches the producer");

    let mut foreign_scope = lookup_request("observe", "corr-lbr-observation-1")?;
    let mut foreign_scope_body: Value =
        serde_json::from_slice(&foreign_scope.body).map_err(|error| error.to_string())?;
    foreign_scope_body["agent_id"] = json!("agent-foreign");
    foreign_scope.body =
        serde_json::to_vec(&foreign_scope_body).map_err(|error| error.to_string())?;
    assert_eq!(service.handle_request(&foreign_scope).0, 409);

    let mut foreign_epoch = lookup_request("observe", "corr-lbr-observation-1")?;
    let mut foreign_epoch_body: Value =
        serde_json::from_slice(&foreign_epoch.body).map_err(|error| error.to_string())?;
    foreign_epoch_body["authority_epoch"] = json!(8);
    foreign_epoch.body =
        serde_json::to_vec(&foreign_epoch_body).map_err(|error| error.to_string())?;
    assert_eq!(service.handle_request(&foreign_epoch).0, 409);

    service
        .game_information_lookup_binding
        .as_mut()
        .ok_or("discovery binding missing")?
        .authority
        .lease_id = String::from("stale-lease");
    assert_eq!(
        service
            .handle_request(&lookup_request("observe", "corr-lbr-observation-1")?)
            .0,
        409
    );
    assert!(service.game_information_lookup_binding.is_none());
    Ok(())
}
