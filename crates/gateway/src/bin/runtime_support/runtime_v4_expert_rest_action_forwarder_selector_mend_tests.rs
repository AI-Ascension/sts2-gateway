// SPDX-License-Identifier: MIT

use super::{RuntimeV4ExpertRestActionForwarder, dispatch, fixture};
use serde_json::{Value, json};

macro_rules! golden {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/",
            $name
        )) as &'static [u8]
    };
}

fn replace_strings(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_strings(item, from, to)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|item| replace_strings(item, from, to)),
        Value::String(text) => *text = text.replace(from, to),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn request_from_response(response: &Value) -> Value {
    let mut request = response.clone();
    request["kind"] = json!("action_request");
    for field in [
        "status",
        "observation",
        "transition",
        "effect_witness",
        "error_code",
    ] {
        request[field] = Value::Null;
    }
    if let Some(before) = response["transition"]["before_generation"].as_u64() {
        request["generation"] = json!(before);
        request["state_id"] = json!(format!("live:{before}"));
    }
    request
}

fn mend_cycle(index: usize) -> Result<Vec<(Value, Value)>, String> {
    let suffix = index.to_string();
    let mut requested: Value =
        serde_json::from_slice(golden!("action-mend-selection-requested.json"))
            .map_err(|error| error.to_string())?;
    for (from, to) in [
        ("rest-op:19:mend", format!("rest-op:19:mend:{suffix}")),
        (
            "rest-option:19:mend",
            format!("rest-option:19:mend:{suffix}"),
        ),
        ("selection:20:mend", format!("selection:20:mend:{suffix}")),
        ("select_player:20", format!("select_player:20:{suffix}")),
        (
            "cancel_selection:20",
            format!("cancel_selection:20:{suffix}"),
        ),
        ("player:local", format!("player:local:{suffix}")),
        ("corr:rest:5", format!("corr:mend:{suffix}:requested")),
    ] {
        replace_strings(&mut requested, from, &to);
    }
    let selector = &requested["transition"]["selector"];
    let selection_id = selector["selection_id"].clone();
    let select = selector["legal_actions"]
        .as_array()
        .and_then(|actions| actions.first())
        .ok_or_else(|| String::from("mend select action missing"))?;
    let select_action_id = select["action_id"].clone();
    let player_id = select["action"]["player_id"]
        .as_str()
        .ok_or_else(|| String::from("mend player choice missing"))?
        .to_owned();

    let mut progressed = requested.clone();
    progressed["operation_id"] = json!(format!("rest-select:20:mend:player:{suffix}"));
    progressed["correlation_id"] = json!(format!("corr:mend:{suffix}:progressed"));
    progressed["generation"] = json!(21);
    progressed["state_id"] = json!("live:21");
    progressed["observation"]["generation"] = json!(21);
    progressed["observation"]["state_id"] = json!("live:21");
    progressed["action"] = json!({
        "action_id": select_action_id,
        "action": {
            "kind": "select_player",
            "selection_id": selection_id,
            "rest_option_id": "mend",
            "player_id": player_id,
        }
    });
    progressed["transition"] = json!({
        "kind": "rest_option_selection_progressed",
        "rest_option_id": "mend",
        "selection_id": selection_id,
        "selection_kind": "player",
        "before_generation": 20,
        "after_generation": 21,
        "required_count": 1,
        "selected_choice_ids": [player_id],
        "remaining_count": 0,
        "selector": {
            "selection_id": selection_id,
            "selection_kind": "player",
            "required_count": 1,
            "selected_choice_ids": [player_id],
            "remaining_count": 0,
            "legal_actions": [
                {
                    "action_id": format!("confirm-selection:21:mend:{suffix}"),
                    "action": {
                        "kind": "confirm_selection",
                        "selection_id": selection_id,
                        "rest_option_id": "mend"
                    }
                },
                {
                    "action_id": format!("cancel-selection:21:mend:{suffix}"),
                    "action": {
                        "kind": "cancel_selection",
                        "selection_id": selection_id,
                        "rest_option_id": "mend"
                    }
                }
            ]
        },
        "effect_witness": null
    });
    progressed["effect_witness"] = Value::Null;

    let mut completed: Value =
        serde_json::from_slice(golden!("action-mend-selection-completed.json"))
            .map_err(|error| error.to_string())?;
    for (from, to) in [
        (
            "rest-select:21:mend:confirm",
            format!("rest-select:21:mend:confirm:{suffix}"),
        ),
        (
            "confirm-selection:21:mend",
            format!("confirm-selection:21:mend:{suffix}"),
        ),
        ("selection:20:mend", format!("selection:20:mend:{suffix}")),
        ("player:local", format!("player:local:{suffix}")),
        ("corr:rest:6", format!("corr:mend:{suffix}:completed")),
    ] {
        replace_strings(&mut completed, from, &to);
    }
    Ok(vec![
        (request_from_response(&requested), requested),
        (request_from_response(&progressed), progressed),
        (request_from_response(&completed), completed),
    ])
}

#[test]
fn alternating_smith_and_mend_lifecycles_reclaim_capacity() -> Result<(), String> {
    let mut forwarder = RuntimeV4ExpertRestActionForwarder::new(16 * 1024, 128 * 1024);
    for index in 0..super::super::selectors::MAX_SELECTOR_ADMISSIONS {
        if index % 2 == 0 {
            for (request_name, response_name) in [
                ("option-request", "requested"),
                ("first-request", "progressed"),
                ("second-request", "second-progressed"),
                ("confirm-request", "completed"),
            ] {
                dispatch(
                    &mut forwarder,
                    &fixture(request_name, index)?,
                    &fixture(response_name, index)?,
                )?;
            }
        } else {
            for (request, response) in mend_cycle(index)? {
                dispatch(&mut forwarder, &request, &response)?;
            }
        }
        assert!(forwarder.selector_admissions.is_empty());
        assert!(forwarder.selector_reservations.is_empty());
    }
    Ok(())
}
