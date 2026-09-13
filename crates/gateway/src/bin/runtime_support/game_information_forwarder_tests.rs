// SPDX-License-Identifier: MIT

use super::super::game_information::GameInformationRoute;
use super::super::game_information_payload::MAX_MESSAGE_BYTES;
use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
const CAPABILITIES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/capabilities-response.json"
));

fn headers(correlation: &str, instance: &str, epoch: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("x-sts2-correlation-id".to_owned(), correlation.to_owned()),
        ("x-sts2-instance-id".to_owned(), instance.to_owned()),
        ("x-sts2-caller-id".to_owned(), "harness".to_owned()),
        ("x-sts2-session-id".to_owned(), "session-1".to_owned()),
        ("x-sts2-lease-id".to_owned(), "lease-1".to_owned()),
        ("x-sts2-lease-epoch".to_owned(), epoch.to_owned()),
    ])
}

fn forwarder() -> GameInformationForwarder {
    GameInformationForwarder::new(MAX_BODY_BYTES, MAX_MESSAGE_BYTES, "content-1", "run-1")
}

#[test]
fn routes_are_exact_and_keyed_by_the_operation() {
    let instance = "instance-1";
    for (method, suffix, expected) in [
        ("GET", "capabilities", GameInformationRoute::Capabilities),
        ("POST", "query", GameInformationRoute::Query),
        ("POST", "list", GameInformationRoute::List),
        ("POST", "search", GameInformationRoute::Search),
        ("POST", "get", GameInformationRoute::Get),
        ("POST", "detail", GameInformationRoute::Detail),
        ("POST", "availability", GameInformationRoute::Availability),
    ] {
        let path = format!("/v1/instances/{instance}/game-information/{suffix}");
        assert_eq!(
            GameInformationRoute::parse(method, &path, instance),
            Some(expected)
        );
    }
    assert_eq!(
        GameInformationRoute::parse(
            "POST",
            "/v1/instances/instance-1/game-information/unknown",
            instance
        ),
        None
    );
    assert_eq!(
        GameInformationRoute::parse(
            "GET",
            "/v1/instances/instance-1/game-information/list",
            instance
        ),
        None
    );
    assert_eq!(
        GameInformationRoute::from_query_kind("list"),
        Some(GameInformationRoute::List)
    );
    assert_eq!(
        GameInformationRoute::from_query_kind("not-allowlisted"),
        None
    );
}

#[test]
fn static_and_live_goldens_validate_with_their_complete_fences() -> Result<(), String> {
    let forwarder = forwarder();
    let static_headers = headers("corr-static-page-1", "instance-1", "1");
    let static_request = forwarder
        .validate_request(GameInformationRoute::List, STATIC_REQUEST, &static_headers)
        .map_err(|error| format!("static request rejected: {error:?}"))?;
    let static_response = forwarder
        .validate_response(
            GameInformationRoute::List,
            Some(&static_request),
            &static_headers,
            200,
            STATIC_RESPONSE,
        )
        .map_err(|error| format!("static response rejected: {error:?}"))?;
    assert!(!static_response.producer_error);

    let canonical_request = forwarder
        .validate_request(GameInformationRoute::Query, STATIC_REQUEST, &static_headers)
        .map_err(|error| format!("canonical request rejected: {error:?}"))?;
    let canonical_response = forwarder
        .validate_response(
            GameInformationRoute::Query,
            Some(&canonical_request),
            &static_headers,
            200,
            STATIC_RESPONSE,
        )
        .map_err(|error| format!("canonical response rejected: {error:?}"))?;
    assert!(!canonical_response.producer_error);

    let live_headers = headers("corr-live-detail", "instance-1", "7");
    let live_request = forwarder
        .validate_request(GameInformationRoute::Detail, LIVE_REQUEST, &live_headers)
        .map_err(|error| format!("live request rejected: {error:?}"))?;
    let live_response = forwarder
        .validate_response(
            GameInformationRoute::Detail,
            Some(&live_request),
            &live_headers,
            200,
            LIVE_RESPONSE,
        )
        .map_err(|error| format!("live response rejected: {error:?}"))?;
    assert_eq!(
        live_response.value["result"]["result_generation"],
        json!(42)
    );
    Ok(())
}

#[test]
fn capabilities_are_validated_without_advertising_over_budget_features() -> Result<(), String> {
    let value: Value = serde_json::from_slice(CAPABILITIES).map_err(|error| error.to_string())?;
    let request_headers = headers("corr-capabilities", "instance-1", "1");
    let result = forwarder().validate_response(
        GameInformationRoute::Capabilities,
        None,
        &request_headers,
        200,
        CAPABILITIES,
    );
    assert!(result.is_ok());

    let mut too_large = value;
    too_large["capabilities"]["limits"]["page_items"] = json!(33);
    let bytes = serde_json::to_vec(&too_large).map_err(|error| error.to_string())?;
    assert!(
        forwarder()
            .validate_response(
                GameInformationRoute::Capabilities,
                None,
                &request_headers,
                200,
                &bytes
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn scope_generation_identity_and_duplicate_input_fail_closed() -> Result<(), String> {
    let request_headers = headers("corr-static-page-1", "instance-1", "1");
    let forwarder = forwarder();
    let mut wrong_scope: Value =
        serde_json::from_slice(STATIC_REQUEST).map_err(|error| error.to_string())?;
    wrong_scope["query"]["binding"]["visibility_scope"] = json!("owner");
    let wrong_scope_bytes = serde_json::to_vec(&wrong_scope).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_request(
            GameInformationRoute::List,
            &wrong_scope_bytes,
            &request_headers
        ),
        Err(GameInformationRequestError::Scope)
    );

    let live_headers = headers("corr-live-detail", "instance-1", "7");
    let mut wrong_generation: Value =
        serde_json::from_slice(LIVE_RESPONSE).map_err(|error| error.to_string())?;
    wrong_generation["result"]["result_generation"] = json!(43);
    let bytes = serde_json::to_vec(&wrong_generation).map_err(|error| error.to_string())?;
    let request = forwarder
        .validate_request(GameInformationRoute::Detail, LIVE_REQUEST, &live_headers)
        .map_err(|error| format!("{error:?}"))?;
    assert!(
        forwarder
            .validate_response(
                GameInformationRoute::Detail,
                Some(&request),
                &live_headers,
                200,
                &bytes
            )
            .is_err()
    );

    let duplicate = br#"{"protocol_version":"game-information-query-v1","protocol_version":"game-information-query-v1"}"#;
    assert_eq!(
        forwarder.validate_request(GameInformationRoute::List, duplicate, &request_headers),
        Err(GameInformationRequestError::Invalid)
    );
    Ok(())
}

#[test]
fn response_budget_and_typed_producer_errors_are_distinct() -> Result<(), String> {
    let request_headers = headers("corr-stale-cursor", "instance-1", "1");
    let mut error: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/game-information-query-v1/golden/error-stale-cursor.json"
    )))
    .map_err(|error| error.to_string())?;
    let error_bytes = serde_json::to_vec(&error).map_err(|error| error.to_string())?;
    let validated = forwarder()
        .validate_response(
            GameInformationRoute::List,
            None,
            &request_headers,
            409,
            &error_bytes,
        )
        .map_err(|error| format!("{error:?}"))?;
    assert!(validated.producer_error);
    error["correlation_id"] = json!("foreign");
    let foreign = serde_json::to_vec(&error).map_err(|error| error.to_string())?;
    assert!(
        forwarder()
            .validate_response(
                GameInformationRoute::List,
                None,
                &request_headers,
                409,
                &foreign,
            )
            .is_err()
    );
    assert_eq!(
        forwarder().validate_response(
            GameInformationRoute::List,
            None,
            &request_headers,
            200,
            &vec![b' '; MAX_MESSAGE_BYTES + 1],
        ),
        Err(GameInformationResponseError::Oversized)
    );
    Ok(())
}
