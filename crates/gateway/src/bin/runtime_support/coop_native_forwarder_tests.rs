// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::super::strict_json;
use super::*;
use serde_json::Value;
use std::collections::BTreeMap;

#[path = "coop_native_forwarder_recovery_tests.rs"]
mod recovery_tests;

const OBSERVATION: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/observation-response.json"
));
const CATALOG_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/legal-catalog-request.json"
));
const CATALOG_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/legal-catalog-response.json"
));
const ACTION_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-settled-request.json"
));
const UNKNOWN_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-unknown-request.json"
));
const ACTION_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-settled-response.json"
));
const UNKNOWN_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-unknown-response.json"
));
const REJOIN_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/rejoin-pending-request.json"
));
const REJOIN_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/rejoin-pending-response.json"
));
const RECOVER_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-recovered-request.json"
));
const RECOVER_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/golden/local-action-recovered-response.json"
));

fn value(bytes: &[u8]) -> Value {
    strict_json::parse(bytes).expect("fixture must be strict JSON")
}

fn headers(value: &Value) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("x-sts2-correlation-id"),
            value["correlation_id"].as_str().unwrap().to_owned(),
        ),
        (
            String::from("x-sts2-instance-id"),
            value["instance_id"].as_str().unwrap().to_owned(),
        ),
        (
            String::from("x-sts2-session-id"),
            value["session_id"].as_str().unwrap().to_owned(),
        ),
        (
            String::from("x-sts2-lease-id"),
            value["lease_id"].as_str().unwrap().to_owned(),
        ),
        (
            String::from("x-sts2-lease-epoch"),
            value["lease_epoch"].to_string(),
        ),
    ])
}

#[test]
fn every_fixed_route_accepts_its_producer_fixture() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let observation = value(OBSERVATION);
    let observation_headers = headers(&observation);
    assert!(
        forwarder
            .validate_request(CoopNativeRoute::Observation, &[], &observation_headers)
            .is_ok()
    );
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Observation,
                None,
                &observation_headers,
                200,
                OBSERVATION,
            )
            .is_ok()
    );

    for (route, request_bytes, response_bytes) in [
        (
            CoopNativeRoute::LegalCatalog,
            CATALOG_REQUEST,
            CATALOG_RESPONSE,
        ),
        (
            CoopNativeRoute::LocalAction,
            ACTION_REQUEST,
            ACTION_RESPONSE,
        ),
        (CoopNativeRoute::Rejoin, REJOIN_REQUEST, REJOIN_RESPONSE),
        (CoopNativeRoute::Recover, RECOVER_REQUEST, RECOVER_RESPONSE),
    ] {
        let request = value(request_bytes);
        let response = value(response_bytes);
        let request_headers = headers(&request);
        let parsed = forwarder
            .validate_request(route, request_bytes, &request_headers)
            .expect("request fixture")
            .expect("body route");
        assert_eq!(parsed, request);
        let response_status = if response["status"] == "rejected" {
            409
        } else {
            200
        };
        assert!(
            forwarder
                .validate_response(
                    route,
                    Some(&request),
                    &request_headers,
                    response_status,
                    response_bytes,
                )
                .is_ok()
        );
    }

    let request = value(UNKNOWN_REQUEST);
    let request_headers = headers(&request);
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LocalAction,
                Some(&request),
                &request_headers,
                200,
                UNKNOWN_RESPONSE,
            )
            .is_ok()
    );
}

#[test]
fn route_parser_is_exact_and_fixed() {
    for (method, suffix, route) in [
        ("GET", "observation", CoopNativeRoute::Observation),
        ("POST", "legal-catalog", CoopNativeRoute::LegalCatalog),
        ("POST", "action", CoopNativeRoute::LocalAction),
        ("POST", "vote", CoopNativeRoute::SharedVote),
        ("POST", "rejoin", CoopNativeRoute::Rejoin),
        ("POST", "recover", CoopNativeRoute::Recover),
    ] {
        let path = format!("/v1/instances/instance-1/coop/native/{suffix}");
        assert_eq!(
            CoopNativeRoute::parse(method, &path, "instance-1"),
            Some(route)
        );
        assert_eq!(CoopNativeRoute::parse("DELETE", &path, "instance-1"), None);
        assert_eq!(CoopNativeRoute::parse(method, &path, "instance-2"), None);
        assert_eq!(
            CoopNativeRoute::parse(method, &format!("{path}/extra"), "instance-1"),
            None
        );
        assert_eq!(
            CoopNativeRoute::parse(method, &format!("{path}?extra"), "instance-1"),
            None
        );
    }
}

#[test]
fn strict_schema_rejects_duplicate_unknown_and_identity_drift() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let request = value(ACTION_REQUEST);
    let request_headers = headers(&request);
    let duplicate = String::from_utf8(ACTION_REQUEST.to_vec())
        .unwrap()
        .replacen(
            "\"operation_id\":\"op:native:action:settled\"",
            "\"operation_id\":\"op:native:action:settled\",\"operation_id\":\"other\"",
            1,
        );
    assert!(
        forwarder
            .validate_request(
                CoopNativeRoute::LocalAction,
                duplicate.as_bytes(),
                &request_headers,
            )
            .is_err()
    );

    let mut unknown = request.clone();
    unknown["unexpected"] = true.into();
    let unknown_bytes = serde_json::to_vec(&unknown).unwrap();
    assert!(
        forwarder
            .validate_request(
                CoopNativeRoute::LocalAction,
                &unknown_bytes,
                &request_headers,
            )
            .is_err()
    );

    let mut wrong_header = request_headers.clone();
    wrong_header.insert(String::from("x-sts2-lease-epoch"), String::from("8"));
    assert!(
        forwarder
            .validate_request(CoopNativeRoute::LocalAction, ACTION_REQUEST, &wrong_header)
            .is_err()
    );
}

#[test]
fn settled_and_recovery_relations_are_fenced() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let request = value(ACTION_REQUEST);
    let mut response = value(ACTION_RESPONSE);
    let request_headers = headers(&request);
    response["effect"]["from_generation"] = 0.into();
    let bytes = serde_json::to_vec(&response).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LocalAction,
                Some(&request),
                &request_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let rejoin_request = value(REJOIN_REQUEST);
    let mut rejoin_response = value(REJOIN_RESPONSE);
    let rejoin_headers = headers(&rejoin_request);
    rejoin_response["recovery"]["kind"] = "reconcile".into();
    let bytes = serde_json::to_vec(&rejoin_response).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Rejoin,
                Some(&rejoin_request),
                &rejoin_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let mut receipt_drift = value(ACTION_RESPONSE);
    receipt_drift["receipt"]["authority_id"] = "authority:forged".into();
    let bytes = serde_json::to_vec(&receipt_drift).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LocalAction,
                Some(&request),
                &request_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let unknown_request = value(UNKNOWN_REQUEST);
    let mut unknown_receipt_drift = value(UNKNOWN_RESPONSE);
    unknown_receipt_drift["receipt"]["state_digest"] =
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into();
    let bytes = serde_json::to_vec(&unknown_receipt_drift).unwrap();
    let unknown_headers = headers(&unknown_request);
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LocalAction,
                Some(&unknown_request),
                &unknown_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let recovered_request = value(RECOVER_REQUEST);
    let mut recovered_receipt_drift = value(RECOVER_RESPONSE);
    recovered_receipt_drift["receipt"]["checkpoint_id"] = "checkpoint:forged".into();
    let bytes = serde_json::to_vec(&recovered_receipt_drift).unwrap();
    let recovered_headers = headers(&recovered_request);
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Recover,
                Some(&recovered_request),
                &recovered_headers,
                200,
                &bytes,
            )
            .is_err()
    );
}

#[test]
fn recovery_request_echo_is_rejected_as_a_response() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let request = value(RECOVER_REQUEST);
    let headers = headers(&request);
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::Recover,
                Some(&request),
                &headers,
                200,
                &bytes,
            )
            .is_err()
    );
}

#[test]
fn catalog_relations_reject_duplicate_ids_and_foreign_voters() {
    let forwarder = CoopNativeForwarder::new(16 * 1024, 128 * 1024);
    let request = value(CATALOG_REQUEST);
    let request_headers = headers(&request);

    let mut duplicate_action = value(CATALOG_RESPONSE);
    duplicate_action["catalog"]["actions"] = serde_json::json!([
        {
            "action_id": "action:native:end-turn",
            "kind": "end_turn",
            "target_peer": null
        },
        {
            "action_id": "action:native:end-turn",
            "kind": "play_card",
            "target_peer": "peer:host1"
        }
    ]);
    let bytes = serde_json::to_vec(&duplicate_action).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LegalCatalog,
                Some(&request),
                &request_headers,
                200,
                &bytes,
            )
            .is_err()
    );

    let mut foreign_vote = value(CATALOG_RESPONSE);
    foreign_vote["catalog"]["votes"] = serde_json::json!([
        {
            "proposal_id": "proposal:native:1",
            "voter_peer": "peer:client1",
            "choice": "yes"
        }
    ]);
    let bytes = serde_json::to_vec(&foreign_vote).unwrap();
    assert!(
        forwarder
            .validate_response(
                CoopNativeRoute::LegalCatalog,
                Some(&request),
                &request_headers,
                200,
                &bytes,
            )
            .is_err()
    );
}

#[test]
fn producer_error_escape_hatch_is_bounded() {
    assert!(CoopNativeForwarder::is_bounded_error(
        409,
        br#"{"error_code":"coop_native_legal_catalog_stale_generation"}"#
    ));
    assert!(!CoopNativeForwarder::is_bounded_error(
        200,
        br#"{"error_code":"ok"}"#
    ));
    assert!(!CoopNativeForwarder::is_bounded_error(
        409,
        br#"{"error_code":"bad-code"}"#
    ));
    assert!(!CoopNativeForwarder::is_bounded_error(
        409,
        br#"{"error_code":"a","extra":true}"#
    ));
}
