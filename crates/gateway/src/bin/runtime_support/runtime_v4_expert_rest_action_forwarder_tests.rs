// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::RuntimeV4ExpertRestActionForwarder;
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

#[path = "runtime_v4_expert_rest_action_forwarder_boundary_tests.rs"]
mod boundary_tests;

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

fn status(value: &Value) -> Option<u16> {
    match value["status"].as_str()? {
        "accepted" => Some(202),
        "settled" => Some(200),
        "rejected" => Some(409),
        "unknown" => Some(404),
        "cancelled" => Some(499),
        _ => None,
    }
}

fn synthetic_request_for_response(response: &Value) -> Value {
    let mut request = response.clone();
    request["kind"] = Value::String(String::from("action_request"));
    request["status"] = Value::Null;
    request["observation"] = Value::Null;
    request["transition"] = Value::Null;
    request["effect_witness"] = Value::Null;
    request["error_code"] = Value::Null;
    if let Some(before_generation) = response["transition"]["before_generation"].as_u64() {
        request["generation"] = json!(before_generation);
        request["state_id"] = Value::String(format!("live:{before_generation}"));
    }
    request
}

#[test]
fn all_candidate_goldens_validate_for_dispatch_and_reconciliation() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);

    let option_request = validate_request_golden(
        &mut forwarder,
        "action-selection-option-request.json",
        golden!("action-selection-option-request.json"),
    )?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-selection-requested.json",
        &option_request,
        golden!("action-selection-requested.json"),
    )?;

    let first_request = validate_request_golden(
        &mut forwarder,
        "action-selection-first-request.json",
        golden!("action-selection-first-request.json"),
    )?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-selection-progressed.json",
        &first_request,
        golden!("action-selection-progressed.json"),
    )?;

    let second_request = validate_request_golden(
        &mut forwarder,
        "action-selection-second-request.json",
        golden!("action-selection-second-request.json"),
    )?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-selection-second-progressed.json",
        &second_request,
        golden!("action-selection-second-progressed.json"),
    )?;

    let confirm_request = validate_request_golden(
        &mut forwarder,
        "action-selection-confirm-request.json",
        golden!("action-selection-confirm-request.json"),
    )?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-selection-completed.json",
        &confirm_request,
        golden!("action-selection-completed.json"),
    )?;

    let basic_request = validate_request_golden(
        &mut forwarder,
        "action-request.json",
        golden!("action-request.json"),
    )?;
    for (name, bytes) in [
        ("action-accepted.json", golden!("action-accepted.json")),
        ("action-completed.json", golden!("action-completed.json")),
        ("action-rejected.json", golden!("action-rejected.json")),
        ("action-unknown.json", golden!("action-unknown.json")),
    ] {
        validate_dispatch_and_reconciliation(&mut forwarder, name, &basic_request, bytes)?;
    }

    // The candidate set has no request golden for Mend. Seed the requested
    // response with a response-shaped option request, then use the retained
    // selector admission to validate its confirmation request.
    let mend_requested: Value =
        serde_json::from_slice(golden!("action-mend-selection-requested.json"))
            .map_err(|error| error.to_string())?;
    let mend_option_request = synthetic_request_for_response(&mend_requested);
    let mend_option_headers = headers(&mend_option_request)?;
    let mend_option_bytes =
        serde_json::to_vec(&mend_option_request).map_err(|error| error.to_string())?;
    let mend_option_request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &mend_option_bytes,
            &mend_option_headers,
        )
        .map_err(|error| format!("synthetic Mend option request: {error:?}"))?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-mend-selection-requested.json",
        &mend_option_request,
        golden!("action-mend-selection-requested.json"),
    )?;

    // The candidate Mend response set omits the typed player-selection
    // response between its requested and completed goldens. Supply that
    // missing wire step so the confirmation request is admitted from the
    // retained catalog instead of weakening the request fence.
    let mend_progress = synthetic_mend_progress_response(&mend_requested);
    let mend_progress_request = synthetic_request_for_response(&mend_progress);
    let mend_progress_headers = headers(&mend_progress_request)?;
    let mend_progress_bytes =
        serde_json::to_vec(&mend_progress_request).map_err(|error| error.to_string())?;
    let mend_progress_request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &mend_progress_bytes,
            &mend_progress_headers,
        )
        .map_err(|error| format!("synthetic Mend selection request: {error:?}"))?;
    let mend_progress_bytes =
        serde_json::to_vec(&mend_progress).map_err(|error| error.to_string())?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "synthetic Mend selection progress",
        &mend_progress_request,
        &mend_progress_bytes,
    )?;

    let mend_completed: Value =
        serde_json::from_slice(golden!("action-mend-selection-completed.json"))
            .map_err(|error| error.to_string())?;
    let mend_confirm_request = synthetic_request_for_response(&mend_completed);
    let mend_confirm_headers = headers(&mend_confirm_request)?;
    let mend_confirm_bytes =
        serde_json::to_vec(&mend_confirm_request).map_err(|error| error.to_string())?;
    let mend_confirm_request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &mend_confirm_bytes,
            &mend_confirm_headers,
        )
        .map_err(|error| format!("synthetic Mend confirmation request: {error:?}"))?;
    validate_dispatch_and_reconciliation(
        &mut forwarder,
        "action-mend-selection-completed.json",
        &mend_confirm_request,
        golden!("action-mend-selection-completed.json"),
    )?;

    // This rejected response intentionally has no admitted request golden:
    // the gateway's request fence rejects an early confirmation before it can
    // be forwarded. Validate the native response shape on the dispatch path.
    let early_confirm: Value =
        serde_json::from_slice(golden!("action-selection-early-confirm-rejected.json"))
            .map_err(|error| error.to_string())?;
    let early_request = synthetic_request_for_response(&early_confirm);
    let early_headers = headers(&early_request)?;
    let response_status = status(&early_confirm).ok_or_else(|| {
        String::from("action-selection-early-confirm-rejected.json: status missing")
    })?;
    let mut dispatch_value = early_confirm;
    dispatch_value["correlation_id"] = early_request["correlation_id"].clone();
    let dispatch_bytes = serde_json::to_vec(&dispatch_value).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &early_request,
            &early_headers,
            response_status,
            &dispatch_bytes,
        )
        .map_err(|error| format!("dispatch early-confirm rejection: {error:?}"))?;
    Ok(())
}

fn synthetic_mend_progress_response(requested: &Value) -> Value {
    let mut response = requested.clone();
    response["operation_id"] = json!("rest-select:20:mend:player");
    response["generation"] = json!(21);
    response["state_id"] = json!("live:21");
    response["action"] = json!({
        "action_id": "select_player:20:mend:player:local",
        "action": {
            "kind": "select_player",
            "selection_id": "selection:20:mend",
            "rest_option_id": "mend",
            "player_id": "player:local"
        }
    });
    response["observation"]["generation"] = json!(21);
    response["observation"]["state_id"] = json!("live:21");
    let mut transition = response["transition"].clone();
    transition["kind"] = json!("rest_option_selection_progressed");
    transition["before_generation"] = json!(20);
    transition["after_generation"] = json!(21);
    transition["selection_id"] = json!("selection:20:mend");
    transition["selection_kind"] = json!("player");
    transition["required_count"] = json!(1);
    transition["selected_choice_ids"] = json!(["player:local"]);
    transition["remaining_count"] = json!(0);
    let mut selector = transition["selector"].clone();
    selector["selected_choice_ids"] = json!(["player:local"]);
    selector["remaining_count"] = json!(0);
    selector["legal_actions"] = json!([
        {
            "action_id": "confirm-selection:21:mend",
            "action": {
                "kind": "confirm_selection",
                "selection_id": "selection:20:mend",
                "rest_option_id": "mend"
            }
        },
        {
            "action_id": "cancel_selection:21:mend",
            "action": {
                "kind": "cancel_selection",
                "selection_id": "selection:20:mend",
                "rest_option_id": "mend"
            }
        }
    ]);
    transition["selector"] = selector;
    response["transition"] = transition;
    response
}

fn validate_request_golden(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    name: &str,
    bytes: &[u8],
) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| format!("{name}: {error}"))?;
    let request_headers = headers(&value)?;
    forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            bytes,
            &request_headers,
        )
        .map_err(|error| format!("{name}: {error:?}"))
}

fn validate_dispatch_and_reconciliation(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    name: &str,
    request: &Value,
    bytes: &[u8],
) -> Result<(), String> {
    let response: Value =
        serde_json::from_slice(bytes).map_err(|error| format!("{name}: {error}"))?;
    let response_status = status(&response).ok_or_else(|| format!("{name}: status missing"))?;
    let request_headers = headers(request)?;
    let mut dispatch_value = response.clone();
    dispatch_value["correlation_id"] = request["correlation_id"].clone();
    let dispatch_bytes =
        serde_json::to_vec(&dispatch_value).map_err(|error| format!("dispatch {name}: {error}"))?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            request,
            &request_headers,
            response_status,
            &dispatch_bytes,
        )
        .map_err(|error| format!("dispatch {name}: {error:?}"))?;

    let operation = response["operation_id"]
        .as_str()
        .ok_or_else(|| format!("{name}: operation missing"))?;
    let response_headers = headers(&response)?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Reconcile(operation.to_owned()),
            &Value::Null,
            &response_headers,
            response_status,
            bytes,
        )
        .map_err(|error| format!("reconcile {name}: {error:?}"))
}

#[test]
fn selector_requests_and_reconciliation_bind_to_the_admitted_catalog() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let option_bytes = golden!("action-selection-option-request.json");
    let option: Value = serde_json::from_slice(option_bytes).map_err(|error| error.to_string())?;
    let option_headers = headers(&option)?;
    let option_request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            option_bytes,
            &option_headers,
        )
        .map_err(|error| format!("option request: {error:?}"))?;
    let requested_bytes = golden!("action-selection-requested.json");
    let requested: Value =
        serde_json::from_slice(requested_bytes).map_err(|error| error.to_string())?;
    let mut dispatch_requested = requested.clone();
    dispatch_requested["correlation_id"] = option_request["correlation_id"].clone();
    let dispatch_requested_bytes =
        serde_json::to_vec(&dispatch_requested).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &option_request,
            &option_headers,
            200,
            &dispatch_requested_bytes,
        )
        .map_err(|error| format!("requested response: {error:?}"))?;

    let mut forged_request: Value =
        serde_json::from_slice(golden!("action-selection-first-request.json"))
            .map_err(|error| error.to_string())?;
    forged_request["action"]["action"]["card_id"] = json!("card:forged");
    let forged_request_bytes =
        serde_json::to_vec(&forged_request).map_err(|error| error.to_string())?;
    let forged_headers = headers(&forged_request)?;
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &forged_request_bytes,
                &forged_headers,
            )
            .is_err()
    );

    let mut stale_request: Value =
        serde_json::from_slice(golden!("action-selection-first-request.json"))
            .map_err(|error| error.to_string())?;
    stale_request["generation"] = json!(11);
    stale_request["state_id"] = json!("live:11");
    let stale_request_bytes =
        serde_json::to_vec(&stale_request).map_err(|error| error.to_string())?;
    let stale_headers = headers(&stale_request)?;
    assert!(
        forwarder
            .validate_request(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &stale_request_bytes,
                &stale_headers,
            )
            .is_err()
    );

    let mut forged_reconciliation = requested;
    forged_reconciliation["transition"]["selector"]["legal_actions"][0]["action_id"] =
        json!("select_card:10:smith:forged");
    let forged_reconciliation_bytes =
        serde_json::to_vec(&forged_reconciliation).map_err(|error| error.to_string())?;
    let requested_headers = headers(&forged_reconciliation)?;
    assert!(
        forwarder
            .validate_response(
                &RuntimeV4ExpertRestActionRoute::Reconcile("rest-op:9:smith".to_owned()),
                &Value::Null,
                &requested_headers,
                200,
                &forged_reconciliation_bytes,
            )
            .is_err()
    );
    Ok(())
}
