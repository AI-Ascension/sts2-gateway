// SPDX-License-Identifier: MIT

use super::super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::{RuntimeV4ExpertRestActionForwardError, RuntimeV4ExpertRestActionForwarder};
use super::{dispatch, fixture, headers, request_only};
use serde_json::Value;
use std::collections::BTreeSet;

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
        assert!(admission.selected_choice_ids.is_empty());
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
        admission.selected_choice_ids,
        BTreeSet::from([String::from("card:1")])
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
        admission.selected_choice_ids,
        BTreeSet::from([String::from("card:1"), String::from("card:2")])
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
        admission.selected_choice_ids,
        BTreeSet::from([String::from("card:1"), String::from("card:2")])
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
fn delayed_first_progress_after_completed_selector_binding_eviction_does_not_resurrect()
-> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let mut open_request = fixture("option-request", 10)?;
    open_request["operation_id"] = "episode-action-10-1".into();
    open_request["correlation_id"] = "probe:smith:open".into();
    let mut open_response = fixture("requested", 10)?;
    open_response["operation_id"] = open_request["operation_id"].clone();
    open_response["correlation_id"] = open_request["correlation_id"].clone();
    dispatch(&mut forwarder, &open_request, &open_response)?;

    // Keep the original card-one operation Accepted by delaying its first GET.
    let mut first_request = fixture("first-request", 10)?;
    first_request["operation_id"] = "episode-action-10-2".into();
    first_request["correlation_id"] = "probe:smith:card-one".into();
    request_only(&mut forwarder, &first_request)
        .map_err(|error| format!("first request: {error:?}"))?;
    let mut accepted = fixture("accepted", 10)?;
    accepted["operation_id"] = first_request["operation_id"].clone();
    accepted["correlation_id"] = first_request["correlation_id"].clone();
    accepted["generation"] = first_request["generation"].clone();
    accepted["state_id"] = first_request["state_id"].clone();
    accepted["action"] = first_request["action"].clone();
    forwarder
        .validate_response(
            &RuntimeV4ExpertRestActionRoute::Dispatch,
            &first_request,
            &headers(&first_request)?,
            202,
            &serde_json::to_vec(&accepted).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("accepted response: {error:?}"))?;

    // Advance the selector through card one, card two, and completion using
    // distinct terminal operations before reconciling the original operation.
    let mut card_one_request = fixture("first-request", 10)?;
    card_one_request["operation_id"] = "episode-action-10-3".into();
    card_one_request["correlation_id"] = "probe:smith:card-one-forward".into();
    let mut card_one_response = fixture("progressed", 10)?;
    card_one_response["operation_id"] = card_one_request["operation_id"].clone();
    card_one_response["correlation_id"] = card_one_request["correlation_id"].clone();
    dispatch(&mut forwarder, &card_one_request, &card_one_response)?;

    let mut card_two_request = fixture("second-request", 10)?;
    card_two_request["operation_id"] = "episode-action-10-4".into();
    card_two_request["correlation_id"] = "probe:smith:card-two".into();
    let mut card_two_response = fixture("second-progressed", 10)?;
    card_two_response["operation_id"] = card_two_request["operation_id"].clone();
    card_two_response["correlation_id"] = card_two_request["correlation_id"].clone();
    dispatch(&mut forwarder, &card_two_request, &card_two_response)?;

    let mut confirm_request = fixture("confirm-request", 10)?;
    confirm_request["operation_id"] = "episode-action-10-5".into();
    confirm_request["correlation_id"] = "probe:smith:confirm".into();
    let mut completed = fixture("completed", 10)?;
    completed["operation_id"] = confirm_request["operation_id"].clone();
    completed["correlation_id"] = confirm_request["correlation_id"].clone();
    completed["effect_witness"]["operation_id"] = confirm_request["operation_id"].clone();
    completed["transition"]["effect_witness"]["operation_id"] =
        confirm_request["operation_id"].clone();
    dispatch(&mut forwarder, &confirm_request, &completed)?;
    assert!(forwarder.selector_admissions.is_empty());
    assert!(
        forwarder
            .operation_bindings
            .get("episode-action-10-2")
            .and_then(|binding| binding.completed_selector.as_ref())
            .is_some()
    );

    // Fill the bounded operation map. The four earlier terminal bindings are
    // evicted in key order, including the completion marker; Accepted card-one
    // remains because it is non-terminal.
    for index in 0..255 {
        let mut request: Value = serde_json::from_slice(golden!("action-request.json"))
            .map_err(|error| error.to_string())?;
        request["operation_id"] = format!("zz-terminal-{index:03}").into();
        request["correlation_id"] = format!("probe:eviction:{index:03}").into();
        request_only(&mut forwarder, &request)
            .map_err(|error| format!("terminal request {index}: {error:?}"))?;
        let mut rejected: Value = serde_json::from_slice(golden!("action-rejected.json"))
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
            "action",
        ] {
            rejected[field] = request[field].clone();
        }
        forwarder
            .validate_response(
                &RuntimeV4ExpertRestActionRoute::Dispatch,
                &request,
                &headers(&request)?,
                409,
                &serde_json::to_vec(&rejected).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("terminal response {index}: {error:?}"))?;
    }

    assert!(
        !forwarder
            .operation_bindings
            .contains_key("episode-action-10-5")
    );
    assert!(
        forwarder
            .operation_bindings
            .contains_key("episode-action-10-2")
    );
    let mut delayed = fixture("progressed", 10)?;
    delayed["operation_id"] = first_request["operation_id"].clone();
    delayed["correlation_id"] = first_request["correlation_id"].clone();
    let route = RuntimeV4ExpertRestActionRoute::Reconcile(String::from("episode-action-10-2"));
    forwarder
        .validate_response(
            &route,
            &Value::Null,
            &headers(&delayed)?,
            200,
            &serde_json::to_vec(&delayed).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("delayed first progress reconciliation: {error:?}"))?;
    assert!(forwarder.selector_admissions.is_empty());
    Ok(())
}

#[test]
fn same_generation_forged_catalog_id_is_rejected() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let mut open_request = fixture("option-request", 10)?;
    open_request["operation_id"] = "forged-open".into();
    open_request["correlation_id"] = "probe:forged:open".into();
    let mut open_response = fixture("requested", 10)?;
    open_response["operation_id"] = open_request["operation_id"].clone();
    open_response["correlation_id"] = open_request["correlation_id"].clone();
    dispatch(&mut forwarder, &open_request, &open_response)?;

    // The selector was admitted at after_generation 10. A reconciliation at
    // that same generation must reproduce its exact catalog; action IDs are
    // opaque and the rejection does not depend on their spelling.
    let mut forged = fixture("requested", 10)?;
    forged["operation_id"] = open_request["operation_id"].clone();
    forged["correlation_id"] = open_request["correlation_id"].clone();
    forged["transition"]["selector"]["legal_actions"][0]["action_id"] =
        "opaque-rewrite:at:same-generation".into();
    let route = RuntimeV4ExpertRestActionRoute::Reconcile(String::from("forged-open"));
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&forged)?,
            200,
            &serde_json::to_vec(&forged).map_err(|error| error.to_string())?,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );
    Ok(())
}
