// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::{
    MAX_OPERATION_BINDINGS, RuntimeV4ExpertRestActionForwardError,
    RuntimeV4ExpertRestActionForwarder,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/",
            $name
        )) as &'static [u8]
    };
}

fn headers(value: &Value) -> Result<BTreeMap<String, String>, String> {
    let mut output = BTreeMap::new();
    for (field, header) in [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ] {
        output.insert(
            String::from(header),
            value[field]
                .as_str()
                .ok_or_else(|| format!("{field} missing"))?
                .to_owned(),
        );
    }
    output.insert(
        String::from("x-sts2-lease-epoch"),
        value["lease_epoch"]
            .as_u64()
            .ok_or_else(|| String::from("lease_epoch missing"))?
            .to_string(),
    );
    Ok(output)
}

fn request_for_operation(operation_id: &str) -> Result<Value, String> {
    let mut request: Value = serde_json::from_slice(golden!("action-request.json"))
        .map_err(|error| error.to_string())?;
    request["operation_id"] = json!(operation_id);
    request["correlation_id"] = json!(format!("corr:{operation_id}"));
    request["action"]["action_id"] = json!(format!("rest-option:{operation_id}:heal"));
    request["generation"] = json!(7);
    request["state_id"] = json!("live:7");
    Ok(request)
}

fn response_for(request: &Value, name: &str) -> Result<Value, String> {
    let mut response: Value = serde_json::from_slice(match name {
        "unknown" => golden!("action-unknown.json"),
        "rejected" => golden!("action-rejected.json"),
        _ => return Err(format!("unsupported response {name}")),
    })
    .map_err(|error| error.to_string())?;
    for field in [
        "operation_id",
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "lease_epoch",
        "generation",
        "state_id",
    ] {
        response[field] = request[field].clone();
    }
    response["action"] = request["action"].clone();
    Ok(response)
}

fn validate_dispatch(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    request: &Value,
    response: &Value,
    status: u16,
) -> Result<(), String> {
    let request_bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    let request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &request_bytes,
            &headers(request)?,
        )
        .map_err(|error| format!("request: {error:?}"))?;
    let response_bytes = serde_json::to_vec(response).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &request,
            &headers(&request)?,
            status,
            &response_bytes,
        )
        .map_err(|error| format!("response: {error:?}"))
}

#[test]
fn unknown_binding_survives_active_capacity_pressure() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let unknown_request = request_for_operation("00-unknown")?;
    let unknown_response = response_for(&unknown_request, "unknown")?;
    validate_dispatch(&mut forwarder, &unknown_request, &unknown_response, 502)?;

    for index in 0..128 {
        let operation_id = format!("op-{index:03}");
        let request = request_for_operation(&operation_id)?;
        let response = response_for(&request, "unknown")?;
        validate_dispatch(&mut forwarder, &request, &response, 502)?;
    }

    let reconcile = RuntimeV4ExpertRestActionRoute::Reconcile(String::from("00-unknown"));
    let response_bytes =
        serde_json::to_vec(&unknown_response).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &reconcile,
            &Value::Null,
            &headers(&unknown_response)?,
            502,
            &response_bytes,
        )
        .map_err(|error| format!("retained unknown reconciliation: {error:?}"))
}

#[test]
fn reconciliation_rejects_same_action_with_forged_generation_or_state() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let request = request_for_operation("forged-reconcile")?;
    let unknown = response_for(&request, "unknown")?;
    validate_dispatch(&mut forwarder, &request, &unknown, 502)?;

    let route = RuntimeV4ExpertRestActionRoute::Reconcile(String::from("forged-reconcile"));
    let forged_generation = {
        let mut response = unknown.clone();
        response["generation"] = json!(8);
        response["state_id"] = json!("live:8");
        response
    };
    let forged_generation_bytes =
        serde_json::to_vec(&forged_generation).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&forged_generation)?,
            502,
            &forged_generation_bytes,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );

    let forged_state = {
        let mut response = unknown.clone();
        response["state_id"] = json!("live:forged");
        response
    };
    let forged_state_bytes =
        serde_json::to_vec(&forged_state).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&forged_state)?,
            502,
            &forged_state_bytes,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );

    let forged_session = {
        let mut response = unknown;
        response["session_id"] = json!("session:forged");
        response
    };
    let forged_session_bytes =
        serde_json::to_vec(&forged_session).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&forged_session)?,
            502,
            &forged_session_bytes,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );
    Ok(())
}

#[test]
fn dispatch_replay_rejects_a_forged_state_identity() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let request = request_for_operation("replay-state")?;
    let request_bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &request_bytes,
            &headers(&request)?,
        )
        .map_err(|error| format!("initial request: {error:?}"))?;

    let mut forged = request;
    forged["state_id"] = json!("live:forged");
    let forged_bytes = serde_json::to_vec(&forged).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &forged_bytes,
            &headers(&forged)?,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed)
    );
    Ok(())
}

#[test]
fn terminal_bindings_are_reclaimed_and_active_capacity_fails_closed() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let terminal_request = request_for_operation("op-000")?;
    let terminal_response = response_for(&terminal_request, "rejected")?;
    validate_dispatch(&mut forwarder, &terminal_request, &terminal_response, 409)?;

    for index in 1..MAX_OPERATION_BINDINGS {
        let operation_id = format!("op-{index:03}");
        let request = request_for_operation(&operation_id)?;
        let request_bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &request_bytes,
                &headers(&request)?,
            )
            .map_err(|error| format!("active operation {operation_id}: {error:?}"))?;
    }
    let replacement = request_for_operation("op-256")?;
    let replacement_bytes = serde_json::to_vec(&replacement).map_err(|error| error.to_string())?;
    forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &replacement_bytes,
            &headers(&replacement)?,
        )
        .map_err(|error| format!("terminal replacement: {error:?}"))?;
    assert_eq!(forwarder.operation_bindings.len(), MAX_OPERATION_BINDINGS);
    assert!(!forwarder.operation_bindings.contains_key("op-000"));

    let rejected = request_for_operation("op-257")?;
    let rejected_bytes = serde_json::to_vec(&rejected).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &rejected_bytes,
            &headers(&rejected)?,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::OperationCapacity)
    );
    Ok(())
}
