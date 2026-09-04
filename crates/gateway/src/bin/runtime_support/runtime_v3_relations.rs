// SPDX-License-Identifier: MIT

use serde_json::Value;

// Structural validation is supplied by the copied canonical schema. These are the
// accompanying neutral cross-field and UTF-8 bounds that JSON Schema cannot express.
pub(super) fn valid(value: &Value) -> bool {
    if !text_bounds(value) {
        return false;
    }
    let observation = &value["observation"];
    if observation.is_object() {
        if observation["generation"] != value["generation"]
            || observation["state_id"] != value["state_id"]
            || !hp_valid(&observation["player"])
        {
            return false;
        }
        if let Some(enemies) = observation["state"]["enemies"].as_array()
            && enemies.iter().any(|enemy| !hp_valid(enemy))
        {
            return false;
        }
    }
    let transition = &value["transition"];
    if transition.is_object()
        && (transition["to_generation"] != value["generation"]
            || transition["state_id"] != value["state_id"]
            || transition["from_generation"].as_u64() >= transition["to_generation"].as_u64())
    {
        return false;
    }
    if let Some(actions) = value["legal_actions"].as_array() {
        for (index, action) in actions.iter().enumerate() {
            if actions[..index]
                .iter()
                .any(|old| old["action_id"] == action["action_id"])
            {
                return false;
            }
        }
    }
    true
}

fn hp_valid(value: &Value) -> bool {
    value["hp"].as_u64() <= value["max_hp"].as_u64()
}

fn text_bounds(value: &Value) -> bool {
    match value {
        Value::String(text) => text.len() <= 512 && !text.chars().any(char::is_control),
        Value::Array(items) => items.iter().all(text_bounds),
        Value::Object(items) => items.values().all(text_bounds),
        _ => true,
    }
}
