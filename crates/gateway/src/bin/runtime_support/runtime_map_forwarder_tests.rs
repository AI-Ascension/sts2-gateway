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

#[test]
fn multibyte_text_is_checked_at_utf8_byte_boundaries() -> Result<(), String> {
    let forwarder = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES);
    for (field, maximum_bytes) in [
        ("game_build", 128_usize),
        ("mod_version", 128),
        ("reason", 256),
    ] {
        let unit = "🦀";
        let exact = unit.repeat(maximum_bytes / unit.len());
        let mut exact_value = response();
        if field == "reason" {
            exact_value["snapshot"]["availability"] = json!("unavailable");
            exact_value["snapshot"]["completeness"] = json!("incomplete");
        }
        exact_value["snapshot"][field] = Value::String(exact.clone());
        assert!(
            forwarder
                .validate_response(
                    RuntimeMapRoute::Snapshot,
                    &headers(),
                    &serde_json::to_vec(&exact_value).map_err(|error| error.to_string())?
                )
                .is_ok(),
            "{field} at the exact UTF-8 byte bound should be accepted"
        );

        let mut overflow_value = response();
        if field == "reason" {
            overflow_value["snapshot"]["availability"] = json!("unavailable");
            overflow_value["snapshot"]["completeness"] = json!("incomplete");
        }
        overflow_value["snapshot"][field] = Value::String(format!("{exact}a"));
        assert!(
            forwarder
                .validate_response(
                    RuntimeMapRoute::Snapshot,
                    &headers(),
                    &serde_json::to_vec(&overflow_value).map_err(|error| error.to_string())?
                )
                .is_err(),
            "{field} one byte over the UTF-8 bound should be rejected"
        );
    }
    Ok(())
}

#[test]
fn control_characters_are_rejected_in_all_bounded_text_fields() -> Result<(), String> {
    let forwarder = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES);
    for control in ['\u{0000}', '\u{007f}', '\u{0080}'] {
        for field in ["game_build", "mod_version", "reason"] {
            let mut value = response();
            if field == "reason" {
                value["snapshot"]["availability"] = json!("unavailable");
                value["snapshot"]["completeness"] = json!("incomplete");
            }
            value["snapshot"][field] = Value::String(format!("prefix{control}suffix"));
            assert!(
                forwarder
                    .validate_response(
                        RuntimeMapRoute::Snapshot,
                        &headers(),
                        &serde_json::to_vec(&value).map_err(|error| error.to_string())?
                    )
                    .is_err(),
                "{field} containing U+{:04X} should be rejected",
                control as u32
            );
        }
    }
    Ok(())
}

#[test]
fn timeout_equal_to_bounds_is_valid_but_exceeding_or_mismatched_is_rejected() -> Result<(), String>
{
    let forwarder = RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES);
    let mut equal = response();
    equal["timeout"]["timeout_millis"] = json!(120_000);
    equal["timeout"]["elapsed_millis"] = json!(120_000);
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&equal).map_err(|error| error.to_string())?
            )
            .is_ok()
    );

    let mut elapsed_over = equal.clone();
    elapsed_over["timeout"]["elapsed_millis"] = json!(120_001);
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&elapsed_over).map_err(|error| error.to_string())?
            )
            .is_err()
    );

    let mut relation_equal = response();
    relation_equal["timeout"]["timeout_millis"] = json!(1);
    relation_equal["timeout"]["elapsed_millis"] = json!(1);
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&relation_equal).map_err(|error| error.to_string())?
            )
            .is_ok()
    );

    let mut relation_over = relation_equal;
    relation_over["timeout"]["elapsed_millis"] = json!(2);
    assert!(
        forwarder
            .validate_response(
                RuntimeMapRoute::Snapshot,
                &headers(),
                &serde_json::to_vec(&relation_over).map_err(|error| error.to_string())?
            )
            .is_err()
    );
    Ok(())
}
