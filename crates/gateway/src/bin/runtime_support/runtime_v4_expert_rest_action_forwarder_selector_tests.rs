// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::{RuntimeV4ExpertRestActionForwardError, RuntimeV4ExpertRestActionForwarder};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[path = "runtime_v4_expert_rest_action_forwarder_selector_mend_tests.rs"]
mod mend_tests;

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
            header.to_owned(),
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

fn rewrite(value: &mut Value, index: usize) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite(item, index)),
        Value::Object(fields) => fields.values_mut().for_each(|item| rewrite(item, index)),
        Value::String(text) => {
            let replacements = [
                ("rest-op:9:smith", format!("rest-op:{index}:smith")),
                (
                    "rest-select:10:smith:card:1",
                    format!("rest-select:{index}:first"),
                ),
                (
                    "rest-select:11:smith:card:2",
                    format!("rest-select:{index}:second"),
                ),
                (
                    "rest-select:12:smith:confirm",
                    format!("rest-select:{index}:confirm"),
                ),
                ("rest-option:9:smith", format!("rest-option:{index}:smith")),
                (
                    "select_card:10:smith:card:1",
                    format!("select_card:{index}:first"),
                ),
                (
                    "select_card:11:smith:card:2",
                    format!("select_card:{index}:second"),
                ),
                (
                    "confirm_selection:12:smith",
                    format!("confirm_selection:{index}"),
                ),
                ("selection:10:smith", format!("selection:{index}:smith")),
                ("corr:rest:2", format!("corr:selector:{index}:2")),
                ("corr:rest:3", format!("corr:selector:{index}:3")),
                ("corr:rest:4", format!("corr:selector:{index}:4")),
                ("corr:rest:5", format!("corr:selector:{index}:5")),
                ("corr:rest:6", format!("corr:selector:{index}:6")),
            ];
            if let Some((_, replacement)) = replacements.iter().find(|(old, _)| text == old) {
                *text = replacement.clone();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn fixture(name: &str, index: usize) -> Result<Value, String> {
    let mut value: Value = serde_json::from_slice(match name {
        "option-request" => golden!("action-selection-option-request.json"),
        "requested" => golden!("action-selection-requested.json"),
        "first-request" => golden!("action-selection-first-request.json"),
        "progressed" => golden!("action-selection-progressed.json"),
        "second-request" => golden!("action-selection-second-request.json"),
        "second-progressed" => golden!("action-selection-second-progressed.json"),
        "confirm-request" => golden!("action-selection-confirm-request.json"),
        "completed" => golden!("action-selection-completed.json"),
        "accepted" => golden!("action-accepted.json"),
        _ => return Err(format!("unknown fixture {name}")),
    })
    .map_err(|error| error.to_string())?;
    rewrite(&mut value, index);
    Ok(value)
}

fn dispatch(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    request: &Value,
    response: &Value,
) -> Result<(), String> {
    let request_bytes = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    let request = forwarder
        .validate_request(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &request_bytes,
            &headers(request)?,
        )
        .map_err(|error| format!("request: {error:?}"))?;
    let mut response = response.clone();
    response["correlation_id"] = request["correlation_id"].clone();
    let response_bytes = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &request,
            &headers(&request)?,
            200,
            &response_bytes,
        )
        .map_err(|error| format!("response: {error:?}"))
}

fn request_only(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    request: &Value,
) -> Result<(), RuntimeV4ExpertRestActionForwardError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|_| RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed)?;
    let request_headers = headers(request)
        .map_err(|_| RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed)?;
    forwarder.validate_request(
        &RuntimeV4ExpertRestActionRoute::Dispatch,
        &bytes,
        &request_headers,
    )?;
    Ok(())
}

#[test]
fn accepted_selector_operation_reconciles_progress_from_retained_before_state() -> Result<(), String>
{
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let mut open_request = fixture("option-request", 10)?;
    open_request["operation_id"] = "episode-action-10-1".into();
    open_request["correlation_id"] = "probe:smith:open".into();
    let mut open_response = fixture("requested", 10)?;
    open_response["operation_id"] = open_request["operation_id"].clone();
    open_response["correlation_id"] = open_request["correlation_id"].clone();
    dispatch(&mut forwarder, &open_request, &open_response)?;
    assert_eq!(forwarder.selector_admissions.len(), 1);
    assert!(forwarder.selector_reservations.is_empty());
    {
        let admission = forwarder
            .selector_admissions
            .get("selection:10:smith")
            .ok_or_else(|| String::from("selector admission missing"))?;
        assert_eq!(admission.generation, 10);
        assert!(admission.selected_choice_ids().is_empty());
    }
    assert_eq!(open_response["generation"].as_u64(), Some(10));
    assert_eq!(
        open_response["transition"]["before_generation"].as_u64(),
        Some(9)
    );

    let mut select_request = fixture("first-request", 10)?;
    select_request["operation_id"] = "episode-action-10-2".into();
    select_request["correlation_id"] = "probe:smith:card-one".into();
    request_only(&mut forwarder, &select_request)
        .map_err(|error| format!("select request: {error:?}"))?;
    let mut accepted = fixture("accepted", 10)?;
    accepted["operation_id"] = select_request["operation_id"].clone();
    accepted["correlation_id"] = select_request["correlation_id"].clone();
    accepted["generation"] = select_request["generation"].clone();
    accepted["state_id"] = select_request["state_id"].clone();
    accepted["action"] = select_request["action"].clone();
    let accepted_bytes = serde_json::to_vec(&accepted).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &select_request,
            &headers(&select_request)?,
            202,
            &accepted_bytes,
        )
        .map_err(|error| format!("accepted response: {error:?}"))?;

    let mut progressed = fixture("progressed", 10)?;
    progressed["operation_id"] = select_request["operation_id"].clone();
    progressed["correlation_id"] = select_request["correlation_id"].clone();
    assert_eq!(
        progressed["transition"]["before_generation"].as_u64(),
        Some(10)
    );
    assert_eq!(
        progressed["transition"]["selected_choice_ids"],
        serde_json::json!(["card:1"])
    );
    let progressed_bytes = serde_json::to_vec(&progressed).map_err(|error| error.to_string())?;
    let route = RuntimeV4ExpertRestActionRoute::Reconcile(
        select_request["operation_id"]
            .as_str()
            .ok_or_else(|| String::from("select operation missing"))?
            .to_owned(),
    );
    for attempt in 0..2 {
        forwarder
            .validate_response(
                &route,
                &Value::Null,
                &headers(&progressed)?,
                200,
                &progressed_bytes,
            )
            .map_err(|error| format!("progress reconciliation {attempt}: {error:?}"))?;
    }
    assert_eq!(forwarder.selector_admissions.len(), 1);
    let admission = forwarder
        .selector_admissions
        .get("selection:10:smith")
        .ok_or_else(|| String::from("progressed selector admission missing"))?;
    assert_eq!(admission.generation, 11);
    assert_eq!(
        admission.selected_choice_ids(),
        &BTreeSet::from([String::from("card:1")])
    );
    assert!(forwarder.selector_reservations.is_empty());

    let mut second_request = fixture("second-request", 10)?;
    second_request["operation_id"] = "episode-action-10-3".into();
    second_request["correlation_id"] = "probe:smith:card-two".into();
    let mut second_progressed = fixture("second-progressed", 10)?;
    second_progressed["operation_id"] = second_request["operation_id"].clone();
    second_progressed["correlation_id"] = second_request["correlation_id"].clone();
    dispatch(&mut forwarder, &second_request, &second_progressed)
        .map_err(|error| format!("second progressed dispatch: {error}"))?;
    let admission = forwarder
        .selector_admissions
        .get("selection:10:smith")
        .ok_or_else(|| String::from("second progressed selector admission missing"))?;
    assert_eq!(admission.generation, 12);
    assert_eq!(
        admission.selected_choice_ids(),
        &BTreeSet::from([String::from("card:1"), String::from("card:2")])
    );

    // The original accepted operation remains a settled replay against its
    // own admission context. Reconciliation must not rewind the shared
    // selector after the second selection has advanced it.
    forwarder
        .validate_response(
            &route,
            &Value::Null,
            &headers(&progressed)?,
            200,
            &progressed_bytes,
        )
        .map_err(|error| format!("delayed progress reconciliation: {error:?}"))?;
    let admission = forwarder
        .selector_admissions
        .get("selection:10:smith")
        .ok_or_else(|| String::from("selector admission rewound"))?;
    assert_eq!(admission.generation, 12);
    assert_eq!(
        admission.selected_choice_ids(),
        &BTreeSet::from([String::from("card:1"), String::from("card:2")])
    );

    let mut forged_count = progressed.clone();
    forged_count["transition"]["selected_choice_ids"] = serde_json::json!(["card:1", "card:2"]);
    forged_count["transition"]["remaining_count"] = 0.into();
    forged_count["transition"]["selector"]["selected_choice_ids"] =
        serde_json::json!(["card:1", "card:2"]);
    forged_count["transition"]["selector"]["remaining_count"] = 0.into();
    forged_count["transition"]["selector"]["legal_actions"] = serde_json::json!([
        {
            "action_id": "confirm_selection:12:smith",
            "action": {
                "kind": "confirm_selection",
                "selection_id": "selection:10:smith",
                "rest_option_id": "smith"
            }
        },
        {
            "action_id": "cancel_selection:12:smith",
            "action": {
                "kind": "cancel_selection",
                "selection_id": "selection:10:smith",
                "rest_option_id": "smith"
            }
        }
    ]);
    let forged_count_bytes =
        serde_json::to_vec(&forged_count).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&forged_count)?,
            200,
            &forged_count_bytes,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );

    let mut confirm_request = fixture("confirm-request", 10)?;
    confirm_request["operation_id"] = "episode-action-10-4".into();
    confirm_request["correlation_id"] = "probe:smith:confirm".into();
    let mut completed = fixture("completed", 10)?;
    completed["operation_id"] = confirm_request["operation_id"].clone();
    completed["correlation_id"] = confirm_request["correlation_id"].clone();
    completed["effect_witness"]["operation_id"] = confirm_request["operation_id"].clone();
    completed["transition"]["effect_witness"]["operation_id"] =
        confirm_request["operation_id"].clone();
    dispatch(&mut forwarder, &confirm_request, &completed)
        .map_err(|error| format!("completion dispatch: {error}"))?;
    assert!(forwarder.selector_admissions.is_empty());

    // A late replay after terminal completion is still readable through its
    // operation binding, but it cannot resurrect the completed selector.
    forwarder
        .validate_response(
            &route,
            &Value::Null,
            &headers(&progressed)?,
            200,
            &progressed_bytes,
        )
        .map_err(|error| format!("late progress reconciliation: {error:?}"))?;
    assert!(forwarder.selector_admissions.is_empty());
    Ok(())
}

#[test]
fn completed_selectors_reclaim_capacity_for_an_unbounded_campaign() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    for index in 0..(super::selectors::MAX_SELECTOR_ADMISSIONS + 1) {
        for (request_name, response_name) in [
            ("option-request", "requested"),
            ("first-request", "progressed"),
            ("second-request", "second-progressed"),
            ("confirm-request", "completed"),
        ] {
            let request = fixture(request_name, index)?;
            let response = fixture(response_name, index)?;
            dispatch(&mut forwarder, &request, &response)?;
        }
        assert!(forwarder.selector_admissions.is_empty());
        assert!(forwarder.selector_reservations.is_empty());
        let completed = fixture("completed", index)?;
        let operation = completed["operation_id"]
            .as_str()
            .ok_or_else(|| String::from("completion operation missing"))?;
        let response_bytes = serde_json::to_vec(&completed).map_err(|error| error.to_string())?;
        forwarder
            .validate_response(
                &RuntimeV4ExpertRestActionRoute::Reconcile(operation.to_owned()),
                &Value::Null,
                &headers(&completed)?,
                200,
                &response_bytes,
            )
            .map_err(|error| format!("completion replay {index}: {error:?}"))?;
    }
    Ok(())
}

#[test]
fn selector_capacity_is_reserved_before_forwarding_and_unknown_recovery_keeps_it()
-> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    for index in 0..super::selectors::MAX_SELECTOR_ADMISSIONS {
        request_only(&mut forwarder, &fixture("option-request", index)?)
            .map_err(|error| format!("reservation {index}: {error:?}"))?;
    }
    let overflow = fixture("option-request", super::selectors::MAX_SELECTOR_ADMISSIONS)?;
    assert_eq!(
        request_only(&mut forwarder, &overflow),
        Err(RuntimeV4ExpertRestActionForwardError::SelectorCapacity)
    );
    assert_eq!(
        forwarder.selector_reservations.len(),
        super::selectors::MAX_SELECTOR_ADMISSIONS
    );

    let first = fixture("option-request", 0)?;
    let operation = first["operation_id"]
        .as_str()
        .ok_or_else(|| String::from("first operation missing"))?
        .to_owned();
    let mut unknown: Value = serde_json::from_slice(golden!("action-unknown.json"))
        .map_err(|error| error.to_string())?;
    unknown["operation_id"] = first["operation_id"].clone();
    unknown["action"] = first["action"].clone();
    unknown["correlation_id"] = first["correlation_id"].clone();
    unknown["instance_id"] = first["instance_id"].clone();
    unknown["session_id"] = first["session_id"].clone();
    unknown["lease_id"] = first["lease_id"].clone();
    unknown["lease_epoch"] = first["lease_epoch"].clone();
    unknown["generation"] = first["generation"].clone();
    unknown["state_id"] = first["state_id"].clone();
    let unknown_bytes = serde_json::to_vec(&unknown).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &first,
            &headers(&first)?,
            502,
            &unknown_bytes,
        )
        .map_err(|error| format!("unknown response: {error:?}"))?;

    let requested = fixture("requested", 0)?;
    let requested_bytes = serde_json::to_vec(&requested).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Reconcile(operation),
            &Value::Null,
            &headers(&requested)?,
            200,
            &requested_bytes,
        )
        .map_err(|error| format!("requested recovery: {error:?}"))?;
    assert_eq!(forwarder.selector_admissions.len(), 1);
    assert_eq!(
        forwarder.selector_reservations.len(),
        super::selectors::MAX_SELECTOR_ADMISSIONS - 1
    );
    Ok(())
}
