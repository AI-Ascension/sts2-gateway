// SPDX-License-Identifier: MIT

use super::super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::{
    RuntimeV4ExpertRestActionForwardError, RuntimeV4ExpertRestActionForwarder, dispatch, fixture,
    headers,
};
use serde_json::{Value, json};

#[path = "runtime_v4_expert_rest_action_forwarder_selector_opaque_lifecycle_tests.rs"]
mod lifecycle_tests;

fn format_selector_action_id(
    namespace: &str,
    generation: u64,
    selection_id: &str,
    kind: &str,
    choice: Option<&str>,
) -> String {
    if namespace == "opaque-arbitrary" {
        let choice = choice.map_or_else(|| kind.to_owned(), |choice| format!("{kind}:{choice}"));
        return format!("opaque-{generation}-{kind}-{}", choice.replace(':', "_"));
    }
    let suffix = choice.map_or_else(|| kind.to_owned(), |choice| format!("{kind}:{choice}"));
    format!("{namespace}:{generation}:{selection_id}:{suffix}")
}

fn set_selector_action_ids(value: &mut Value, namespace: &str) -> Result<(), String> {
    let generation = value["transition"]["after_generation"]
        .as_u64()
        .ok_or_else(|| String::from("selector generation missing"))?;
    let selection_id = value["transition"]["selector"]["selection_id"]
        .as_str()
        .ok_or_else(|| String::from("selector ID missing"))?
        .to_owned();
    let legal_actions = value["transition"]["selector"]["legal_actions"]
        .as_array_mut()
        .ok_or_else(|| String::from("selector legal actions missing"))?;
    for legal in legal_actions {
        let action = legal
            .get("action")
            .ok_or_else(|| String::from("selector action missing"))?;
        let kind = action["kind"]
            .as_str()
            .ok_or_else(|| String::from("selector action kind missing"))?;
        let choice = action
            .get("card_id")
            .or_else(|| action.get("player_id"))
            .and_then(Value::as_str);
        legal["action_id"] = json!(format_selector_action_id(
            namespace,
            generation,
            &selection_id,
            kind,
            choice,
        ));
    }
    Ok(())
}

fn set_request_action_id(value: &mut Value, namespace: &str) -> Result<(), String> {
    let action = &value["action"]["action"];
    if action.get("selection_id").is_none() {
        return Ok(());
    }
    let generation = value["generation"]
        .as_u64()
        .ok_or_else(|| String::from("request generation missing"))?;
    let selection_id = action["selection_id"]
        .as_str()
        .ok_or_else(|| String::from("request selection ID missing"))?;
    let kind = action["kind"]
        .as_str()
        .ok_or_else(|| String::from("request action kind missing"))?;
    let choice = action
        .get("card_id")
        .or_else(|| action.get("player_id"))
        .and_then(Value::as_str);
    value["action"]["action_id"] = json!(format_selector_action_id(
        namespace,
        generation,
        selection_id,
        kind,
        choice,
    ));
    Ok(())
}

fn replace_exact(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_exact(item, from, to)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|item| replace_exact(item, from, to)),
        Value::String(text) if text == from => *text = to.to_owned(),
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn shift_generation(value: &mut Value, offset: u64) -> Result<(), String> {
    let generation = value["generation"]
        .as_u64()
        .ok_or_else(|| String::from("generation missing"))?
        .checked_add(offset)
        .ok_or_else(|| String::from("generation overflow"))?;
    value["generation"] = json!(generation);
    value["state_id"] = json!(format!("live:{generation}"));
    if value["observation"].is_object() {
        value["observation"]["generation"] = json!(generation);
        value["observation"]["state_id"] = json!(format!("live:{generation}"));
    }
    if value["transition"].is_object() {
        for field in ["before_generation", "after_generation"] {
            let shifted = value["transition"][field]
                .as_u64()
                .ok_or_else(|| format!("transition {field} missing"))?
                .checked_add(offset)
                .ok_or_else(|| String::from("transition generation overflow"))?;
            value["transition"][field] = json!(shifted);
        }
        if value["effect_witness"].is_object() {
            value["effect_witness"]["generation"] = json!(generation);
        }
        if value["transition"]["effect_witness"].is_object() {
            value["transition"]["effect_witness"]["generation"] = json!(generation);
        }
    }
    Ok(())
}

fn set_lifecycle(value: &mut Value, session_id: &str, lease_epoch: u64) {
    value["instance_id"] = json!("instance:selector-test");
    value["session_id"] = json!(session_id);
    value["lease_id"] = json!("lease:selector-test");
    value["lease_epoch"] = json!(lease_epoch);
}

fn prepare(
    mut value: Value,
    index: usize,
    generation_offset: u64,
    reuse_selection_id: bool,
    namespace: &str,
    session_id: &str,
    lease_epoch: u64,
) -> Result<Value, String> {
    if generation_offset != 0 {
        shift_generation(&mut value, generation_offset)?;
    }
    if reuse_selection_id {
        replace_exact(
            &mut value,
            &format!("selection:{index}:smith"),
            "selection:0:smith",
        );
    }
    set_lifecycle(&mut value, session_id, lease_epoch);
    if value["transition"]["selector"].is_object() {
        set_selector_action_ids(&mut value, namespace)?;
    }
    set_request_action_id(&mut value, namespace)?;
    Ok(value)
}

fn run_cycle(
    forwarder: &mut RuntimeV4ExpertRestActionForwarder,
    index: usize,
    generation_offset: u64,
    reuse_selection_id: bool,
    namespace: &str,
    session_id: &str,
    lease_epoch: u64,
) -> Result<(), String> {
    let open_request = prepare(
        fixture("option-request", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    let requested = prepare(
        fixture("requested", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    dispatch(forwarder, &open_request, &requested)?;

    let first_request = prepare(
        fixture("first-request", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    let mut progressed = prepare(
        fixture("progressed", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    progressed["action"]["action_id"] = first_request["action"]["action_id"].clone();
    dispatch(forwarder, &first_request, &progressed)?;

    let second_request = prepare(
        fixture("second-request", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    let mut second_progressed = prepare(
        fixture("second-progressed", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    second_progressed["action"]["action_id"] = second_request["action"]["action_id"].clone();
    dispatch(forwarder, &second_request, &second_progressed)?;

    let confirm_request = prepare(
        fixture("confirm-request", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    let mut completed = prepare(
        fixture("completed", index)?,
        index,
        generation_offset,
        reuse_selection_id,
        namespace,
        session_id,
        lease_epoch,
    )?;
    completed["action"]["action_id"] = confirm_request["action"]["action_id"].clone();
    completed["effect_witness"]["operation_id"] = completed["operation_id"].clone();
    completed["transition"]["effect_witness"]["operation_id"] = completed["operation_id"].clone();
    dispatch(forwarder, &confirm_request, &completed)
}

#[test]
fn native_shaped_selector_ids_are_accepted_across_generations() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    run_cycle(
        &mut forwarder,
        0,
        0,
        false,
        "rest-selection",
        "session:native-selector",
        9,
    )?;
    assert!(forwarder.selector_admissions.is_empty());
    Ok(())
}

#[test]
fn arbitrary_opaque_selector_ids_without_colons_are_accepted() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    run_cycle(
        &mut forwarder,
        0,
        0,
        false,
        "opaque-arbitrary",
        "session:arbitrary",
        6,
    )?;
    run_cycle(
        &mut forwarder,
        1,
        10,
        true,
        "opaque-arbitrary",
        "session:arbitrary",
        6,
    )?;
    assert!(forwarder.selector_admissions.is_empty());
    Ok(())
}

#[test]
fn opaque_namespaced_selector_ids_and_reused_ids_are_lifecycle_scoped() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    run_cycle(
        &mut forwarder,
        0,
        0,
        false,
        "opaque-producer",
        "session:first",
        4,
    )?;
    run_cycle(
        &mut forwarder,
        1,
        0,
        true,
        "opaque-producer",
        "session:second",
        5,
    )?;
    run_cycle(
        &mut forwarder,
        2,
        10,
        true,
        "opaque-producer",
        "session:second",
        5,
    )?;
    assert!(forwarder.selector_admissions.is_empty());
    Ok(())
}

#[test]
fn same_generation_reconciliation_requires_the_stored_catalog() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    let open_request = fixture("option-request", 0)?;
    let requested = fixture("requested", 0)?;
    dispatch(&mut forwarder, &open_request, &requested)?;
    let operation_id = open_request["operation_id"]
        .as_str()
        .ok_or_else(|| String::from("operation ID missing"))?
        .to_owned();
    let route = RuntimeV4ExpertRestActionRoute::Reconcile(operation_id);
    let exact = serde_json::to_vec(&requested).map_err(|error| error.to_string())?;
    forwarder
        .validate_response(&route, &Value::Null, &headers(&requested)?, 200, &exact)
        .map_err(|error| format!("exact catalog replay: {error:?}"))?;

    let mut rewritten = requested;
    rewritten["transition"]["selector"]["legal_actions"][0]["action_id"] =
        json!("select_card:10:smith:rewritten");
    let rewritten_bytes = serde_json::to_vec(&rewritten).map_err(|error| error.to_string())?;
    assert_eq!(
        forwarder.validate_response(
            &route,
            &Value::Null,
            &headers(&rewritten)?,
            200,
            &rewritten_bytes,
        ),
        Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)
    );
    Ok(())
}
