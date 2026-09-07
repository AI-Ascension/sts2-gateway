// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert::RuntimeV4ExpertRoute;
use super::RuntimeV4ExpertForwarder;
use std::collections::BTreeMap;

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("x-sts2-instance-id".into(), "instance-1".into()),
        ("x-sts2-session-id".into(), "session-1".into()),
        ("x-sts2-lease-id".into(), "lease-1".into()),
        ("x-sts2-lease-epoch".into(), "1".into()),
        ("x-sts2-correlation-id".into(), "request-1".into()),
    ])
}

#[test]
fn route_requires_an_empty_get_body_and_accepts_the_canonical_observation() {
    let forwarder = RuntimeV4ExpertForwarder::new(16 * 1024, 128 * 1024);
    let route = RuntimeV4ExpertRoute::State;
    assert!(forwarder.validate_request(&route, &[], &headers()).is_ok());
    assert!(
        forwarder
            .validate_request(&route, b"{}", &headers())
            .is_err()
    );
    let golden = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ));
    assert!(
        forwarder
            .validate_response(&route, &serde_json::Value::Null, &headers(), golden)
            .is_ok()
    );
}

#[test]
fn malformed_or_wrong_digest_observations_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV4ExpertForwarder::new(16 * 1024, 128 * 1024);
    let route = RuntimeV4ExpertRoute::State;
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))?;
    value["schema_digest"] = serde_json::Value::String(String::from("0").repeat(64));
    let bytes = serde_json::to_vec(&value)?;
    assert!(
        forwarder
            .validate_response(&route, &serde_json::Value::Null, &headers(), &bytes)
            .is_err()
    );
    Ok(())
}

#[test]
fn action_request_and_settlement_are_fenced_to_the_route_operation()
-> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV4ExpertForwarder::new(16 * 1024, 128 * 1024);
    let dispatch = RuntimeV4ExpertRoute::Dispatch;
    let request = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-request.json"
    ));
    let value = forwarder
        .validate_request(&dispatch, request, &headers())
        .map_err(|error| format!("action request must validate: {error:?}"))?;
    assert_eq!(value["operation_id"], "potion-op-1");

    let response = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
    ));
    let mut settled: serde_json::Value = serde_json::from_slice(response)?;
    settled["correlation_id"] = serde_json::Value::String("request-1".into());
    settled["observation"] = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))?;
    assert!(
        forwarder
            .validate_response(
                &RuntimeV4ExpertRoute::Reconcile("potion-op-1".into()),
                &value,
                &headers(),
                &serde_json::to_vec(&settled)?,
            )
            .is_ok()
    );

    let wrong_operation = RuntimeV4ExpertRoute::Reconcile("other-op".into());
    assert!(
        forwarder
            .validate_response(
                &wrong_operation,
                &value,
                &headers(),
                &serde_json::to_vec(&settled)?,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn route_parser_rejects_path_traversal_and_assigns_mutation_scope() {
    assert_eq!(
        RuntimeV4ExpertRoute::parse("GET", "/v4/instances/instance-1/expert-state", "instance-1"),
        Some(RuntimeV4ExpertRoute::State)
    );
    assert_eq!(
        RuntimeV4ExpertRoute::parse(
            "POST",
            "/v4/instances/instance-1/expert-action",
            "instance-1"
        ),
        Some(RuntimeV4ExpertRoute::Dispatch)
    );
    assert_eq!(
        RuntimeV4ExpertRoute::parse(
            "GET",
            "/v4/instances/instance-1/expert-actions/potion-op-1",
            "instance-1"
        ),
        Some(RuntimeV4ExpertRoute::Reconcile("potion-op-1".into()))
    );
    assert!(
        RuntimeV4ExpertRoute::parse(
            "GET",
            "/v4/instances/instance-1/expert-actions/../other",
            "instance-1"
        )
        .is_none()
    );
}
