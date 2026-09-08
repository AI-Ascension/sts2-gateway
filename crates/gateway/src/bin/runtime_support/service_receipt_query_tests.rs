// SPDX-License-Identifier: MIT

use super::receipt_query::{validate_request, validate_response};
use super::test_support::{authenticated_request, test_service};
use super::{read_request, write_response};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

const REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-request.json"
));
const ACCEPTED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-accepted.json"
));
const SETTLED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
));
const REJECTED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-rejected.json"
));
const UNKNOWN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-unknown.json"
));

fn request_headers() -> Result<BTreeMap<String, String>, String> {
    let value: Value = serde_json::from_slice(REQUEST).map_err(|error| error.to_string())?;
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("x-sts2-instance-id"),
        value["instance_id"]
            .as_str()
            .ok_or_else(|| String::from("instance_id missing"))?
            .to_owned(),
    );
    headers.insert(
        String::from("x-sts2-session-id"),
        value["session_id"]
            .as_str()
            .ok_or_else(|| String::from("session_id missing"))?
            .to_owned(),
    );
    headers.insert(
        String::from("x-sts2-lease-id"),
        value["lease_id"]
            .as_str()
            .ok_or_else(|| String::from("lease_id missing"))?
            .to_owned(),
    );
    headers.insert(
        String::from("x-sts2-correlation-id"),
        value["correlation_id"]
            .as_str()
            .ok_or_else(|| String::from("correlation_id missing"))?
            .to_owned(),
    );
    headers.insert(
        String::from("x-sts2-lease-epoch"),
        value["lease_epoch"]
            .as_u64()
            .ok_or_else(|| String::from("lease_epoch missing"))?
            .to_string(),
    );
    Ok(headers)
}

fn replace(body: &[u8], from: &str, to: &str) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    if text.matches(from).count() != 1 {
        return Err(format!("expected one occurrence of {from:?}"));
    }
    Ok(text.replacen(from, to, 1).into_bytes())
}

#[test]
fn all_frozen_golden_statuses_validate_against_the_exact_neutral_profile() -> Result<(), String> {
    let headers = request_headers()?;
    let request = validate_request(REQUEST, &headers).map_err(|error| format!("{error:?}"))?;
    for (body, status) in [
        (ACCEPTED, 200),
        (SETTLED, 200),
        (REJECTED, 409),
        (UNKNOWN, 503),
    ] {
        validate_response(&request, status, body).map_err(|error| format!("{error:?}"))?;
    }
    Ok(())
}

#[test]
fn malformed_and_semantically_invalid_vectors_fail_closed() -> Result<(), String> {
    let headers = request_headers()?;
    let request_value =
        validate_request(REQUEST, &headers).map_err(|error| format!("{error:?}"))?;

    let invalid_request = replace(
        REQUEST,
        "\"participant_ids\":[\"peer-1\",\"peer-2\"]",
        "\"participant_ids\":[\"peer-2\",\"peer-3\"]",
    )?;
    assert!(validate_request(&invalid_request, &headers).is_err());

    let unsorted = replace(
        REQUEST,
        "\"participant_ids\":[\"peer-1\",\"peer-2\"]",
        "\"participant_ids\":[\"peer-2\",\"peer-1\"]",
    )?;
    assert!(validate_request(&unsorted, &headers).is_err());

    let generation_mismatch = replace(
        REQUEST,
        "\"expected_host_generation\":17",
        "\"expected_host_generation\":18",
    )?;
    assert!(validate_request(&generation_mismatch, &headers).is_err());

    let duplicate = replace(
        REQUEST,
        "\"error_code\":null",
        "\"error_code\":null,\"error_code\":null",
    )?;
    assert!(validate_request(&duplicate, &headers).is_err());

    let numeric_spelling = replace(REQUEST, "\"lease_epoch\":9", "\"lease_epoch\":9.0")?;
    assert!(validate_request(&numeric_spelling, &headers).is_err());

    let fresh_scope = replace(
        SETTLED,
        "\"evidence_scope\":\"retained_receipt\"",
        "\"evidence_scope\":\"fresh_reconciliation\"",
    )?;
    assert!(validate_response(&request_value, 200, &fresh_scope).is_err());

    let status_mismatch = replace(
        SETTLED,
        "\"status\":\"settled\",\"after_host_generation\":18",
        "\"status\":\"accepted\",\"after_host_generation\":18",
    )?;
    assert!(validate_response(&request_value, 200, &status_mismatch).is_err());

    let generation_not_advanced = replace(
        SETTLED,
        "\"after_host_generation\":18",
        "\"after_host_generation\":17",
    )?;
    assert!(validate_response(&request_value, 200, &generation_not_advanced).is_err());

    let effect_kind_mismatch = replace(
        SETTLED,
        "\"effect_kind\":\"play_card_settled\"",
        "\"effect_kind\":\"end_turn_settled\"",
    )?;
    assert!(validate_response(&request_value, 200, &effect_kind_mismatch).is_err());

    let response_identity_mismatch = replace(
        SETTLED,
        "\"effect_id\":\"effect:op:run-17:0001\"",
        "\"effect_id\":\"effect:op:run-17:0002\"",
    )?;
    assert!(validate_response(&request_value, 200, &response_identity_mismatch).is_err());
    Ok(())
}

#[test]
fn route_requires_content_type_and_active_lease_before_downstream() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request(&service.coop_receipt_query_path());
    request.method = String::from("POST");
    request.body = REQUEST.to_vec();
    assert_eq!(service.handle_request(&request).0, 400);
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    service.lease_active = false;
    assert_eq!(service.handle_request(&request).0, 409);
    Ok(())
}

#[test]
fn route_forwards_only_the_fixed_path_and_returns_a_validated_receipt() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = thread::spawn(move || -> Result<(), String> {
        let expires = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < expires => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let forwarded = read_request(&mut stream).map_err(|error| error.to_string())?;
        if forwarded.method != "POST"
            || forwarded.path != "/api/v1/coop/native/receipt-query"
            || forwarded.body != REQUEST
            || forwarded.headers.get("authorization").map(String::as_str)
                != Some("Bearer mod-token")
            || forwarded
                .headers
                .get("x-sts2-session-id")
                .map(String::as_str)
                != Some("session-native-17")
            || forwarded
                .headers
                .get("x-sts2-lease-epoch")
                .map(String::as_str)
                != Some("9")
        {
            return Err(String::from("forwarded request changed at the gateway"));
        }
        write_response(&mut stream, 200, SETTLED).map_err(|error| error.to_string())?;
        Ok(())
    });

    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.session_id = String::from("session-native-17");
    service.config.lease_epoch = 9;
    let mut request = authenticated_request(&service.coop_receipt_query_path());
    request.method = String::from("POST");
    request.headers.insert(
        String::from("x-sts2-session-id"),
        String::from("session-native-17"),
    );
    request
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("9"));
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr:17:1"),
    );
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = REQUEST.to_vec();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    assert_eq!(body, SETTLED);
    worker
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    Ok(())
}
