// SPDX-License-Identifier: MIT

use super::super::runtime_map::RuntimeMapRoute;
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("x-sts2-correlation-id"),
            String::from("corr-42"),
        ),
        (
            String::from("x-sts2-instance-id"),
            String::from("instance-1"),
        ),
        (String::from("x-sts2-session-id"), String::from("session-1")),
        (String::from("x-sts2-lease-id"), String::from("lease-1")),
        (String::from("x-sts2-lease-epoch"), String::from("7")),
    ])
}

fn response() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-map-v1/golden/snapshot-response.json"
    )))
    .unwrap_or(Value::Null)
}

#[test]
fn route_is_exact_and_read_only() {
    let path = "/v1/instances/instance-1/map-snapshot";
    assert_eq!(
        RuntimeMapRoute::parse("GET", path, "instance-1"),
        Some(RuntimeMapRoute::Snapshot)
    );
    assert_eq!(RuntimeMapRoute::parse("POST", path, "instance-1"), None);
    assert_eq!(
        RuntimeMapRoute::parse(
            "GET",
            "/v1/instances/instance-1/map-snapshot/extra",
            "instance-1"
        ),
        None
    );
}

#[test]
fn valid_map_response_preserves_identity_and_topology_bounds() -> Result<(), String> {
    let encoded = serde_json::to_vec(&response()).map_err(|error| error.to_string())?;
    let parsed = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES)
        .validate_response(RuntimeMapRoute::Snapshot, &headers(), &encoded)
        .map_err(|error| format!("unexpected rejection: {error:?}"))?;
    assert_eq!(parsed["generation"], 42);
    assert_eq!(
        parsed["snapshot"]["edges"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(
        parsed["snapshot"]["bindings"].as_array().map(Vec::len),
        Some(2)
    );
    Ok(())
}

#[test]
fn mismatched_identity_unknown_fields_duplicates_and_oversized_maps_fail_closed()
-> Result<(), String> {
    let forwarder = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES);
    let mut wrong = response();
    wrong["lease_epoch"] = 8.into();
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&wrong).map_err(|error| error.to_string())?
            )
            .is_err()
    );

    let mut unknown = response();
    unknown["private_host_state"] = true.into();
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&unknown).map_err(|error| error.to_string())?
            )
            .is_err()
    );

    let mut too_many = response();
    too_many["snapshot"]["nodes"] = Value::Array(
        (0..=MAX_MAP_NODES)
            .map(|index| {
                json!({
                    "id": format!("node-{index}"),
                    "row": index as i64,
                    "column": index as i64,
                    "category": "other",
                    "visited": false
                })
            })
            .collect(),
    );
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&too_many).map_err(|error| error.to_string())?
            )
            .is_err()
    );

    let mut duplicate = serde_json::to_string(&response()).map_err(|error| error.to_string())?;
    duplicate = duplicate.replacen(
        "\"protocol_version\":\"runtime-map-v1\",",
        "\"protocol_version\":\"runtime-map-v1\",\"protocol_version\":\"runtime-map-v1\",",
        1,
    );
    assert!(
        forwarder
            .validate_response(RuntimeMapRoute::Snapshot, &headers(), duplicate.as_bytes())
            .is_err()
    );
    Ok(())
}

#[test]
fn generation_and_binding_relations_are_fenced() -> Result<(), String> {
    let forwarder = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES);
    let mut wrong_generation = response();
    wrong_generation["snapshot"]["generation"] = 41.into();
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&wrong_generation).map_err(|error| error.to_string())?
            )
            .is_err()
    );

    let mut wrong_binding = response();
    wrong_binding["snapshot"]["bindings"][1]["action"]["node_id"] =
        Value::String(String::from("map-option:42:left"));
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&wrong_binding).map_err(|error| error.to_string())?
            )
            .is_err()
    );
    Ok(())
}
