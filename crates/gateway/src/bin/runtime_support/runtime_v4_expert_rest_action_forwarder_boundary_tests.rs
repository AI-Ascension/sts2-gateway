// SPDX-License-Identifier: MIT

use super::super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::RuntimeV4ExpertRestActionForwarder;
use super::headers;
use serde_json::Value;

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/",
            $name
        )) as &'static [u8]
    };
}

#[test]
fn request_and_route_guards_are_fixed_and_body_bounded() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(32, 128 * 1024);
    let bytes = golden!("action-request.json");
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let request_headers = headers(&value)?;
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                bytes,
                &request_headers
            )
            .is_err()
    );
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Reconcile("rest-op:7:heal".into()),
                &[],
                &request_headers
            )
            .is_ok()
    );
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Reconcile("rest-op:7:heal".into()),
                b"{}",
                &request_headers
            )
            .is_err()
    );
    assert_eq!(
        RuntimeV4ExpertRestActionRoute::parse(
            "POST",
            "/v4/instances/instance-1/expert-rest-action",
            "instance-1"
        ),
        Some(RuntimeV4ExpertRestActionRoute::Dispatch)
    );
    assert_eq!(
        RuntimeV4ExpertRestActionRoute::parse(
            "GET",
            "/v4/instances/instance-1/expert-rest-actions/rest-op:7:heal",
            "instance-1"
        ),
        Some(RuntimeV4ExpertRestActionRoute::Reconcile(
            "rest-op:7:heal".into()
        ))
    );
    assert!(
        RuntimeV4ExpertRestActionRoute::parse(
            "GET",
            "/v4/instances/instance-1/expert-rest-actions/../secret",
            "instance-1"
        )
        .is_none()
    );
    let mut long_identity_headers = request_headers.clone();
    let long_identity = "x".repeat(129);
    long_identity_headers.insert(String::from("x-sts2-instance-id"), long_identity.clone());
    let mut long_identity_value = value.clone();
    long_identity_value["instance_id"] = Value::String(long_identity);
    let long_identity_body =
        serde_json::to_vec(&long_identity_value).map_err(|error| error.to_string())?;
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &long_identity_body,
                &long_identity_headers,
            )
            .is_err()
    );
    let long_operation = "x".repeat(129);
    let mut long_operation_value = value.clone();
    long_operation_value["operation_id"] = Value::String(long_operation);
    let long_operation_body =
        serde_json::to_vec(&long_operation_value).map_err(|error| error.to_string())?;
    let mut identity_bounded = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    assert!(
        identity_bounded
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &long_operation_body,
                &request_headers,
            )
            .is_err()
    );
    let mut permissive = RuntimeV4ExpertRestActionForwarder::new(128 * 1024, 128 * 1024);
    assert!(
        permissive
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &vec![b'x'; 16 * 1024 + 1],
                &request_headers,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn response_status_and_identity_are_fenced() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let request_bytes = golden!("action-request.json");
    let request_value: Value =
        serde_json::from_slice(request_bytes).map_err(|error| error.to_string())?;
    let request_headers = headers(&request_value)?;
    let response = golden!("action-completed.json");
    assert!(
        forwarder
            .validate_response(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &request_value,
                &request_headers,
                202,
                response
            )
            .is_err()
    );
    let mut wrong_headers = request_headers.clone();
    wrong_headers.insert(String::from("x-sts2-correlation-id"), String::from("other"));
    assert!(
        forwarder
            .validate_response(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &request_value,
                &wrong_headers,
                200,
                response
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn native_success_status_maps_to_candidate_http_status() -> Result<(), String> {
    let forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    for (envelope_status, candidate_status) in [
        ("accepted", 202),
        ("settled", 200),
        ("rejected", 409),
        ("unknown", 404),
        ("cancelled", 499),
    ] {
        let body = format!(r#"{{"status":"{envelope_status}"}}"#);
        assert_eq!(
            forwarder.candidate_http_status(200, body.as_bytes()),
            Ok(candidate_status),
            "native 200 mapping for {envelope_status}"
        );
    }
    assert_eq!(
        forwarder.candidate_http_status(503, br#"{"status":"unknown"}"#),
        Ok(502)
    );
    assert_eq!(
        forwarder.candidate_http_status(504, br#"{"status":"unknown"}"#),
        Ok(504)
    );
    assert!(
        forwarder
            .candidate_http_status(503, br#"{"status":"settled"}"#)
            .is_err()
    );
    assert!(
        forwarder
            .candidate_http_status(202, br#"{"status":"settled"}"#)
            .is_err()
    );
    Ok(())
}
