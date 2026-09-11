// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;

#[path = "runtime_v4_expert_rest_action_selection.rs"]
mod selection;
pub(super) use selection::{SelectorAdmission, SelectorLifecycle, admission_from_transition};
use selection::{completed_selection_valid, selector_valid};

const MAX_GENERATION: u64 = 9_007_199_254_740_991;

pub(super) fn response_valid(
    value: &Value,
    request: &Value,
    route_operation: Option<&str>,
    status_code: u16,
    admissions: &BTreeMap<String, SelectorAdmission>,
    observation_valid: impl Fn(&Value) -> bool,
) -> bool {
    let Some(status) = value["status"].as_str() else {
        return false;
    };
    if !http_status_matches(status, status_code)
        || !operation_matches(value, request, route_operation)
        || !action_matches_request(value, request)
        || !generation_shape(value, request, status)
    {
        return false;
    }
    match status {
        "accepted" => accepted(value),
        "settled" => settled(value, request, admissions, observation_valid),
        "rejected" | "unknown" | "cancelled" => rejected_or_uncertain(value),
        _ => false,
    }
}

fn http_status_matches(status: &str, code: u16) -> bool {
    match status {
        "accepted" => code == 202,
        "settled" => code == 200,
        "rejected" => matches!(code, 400 | 409),
        "unknown" => matches!(code, 404 | 408 | 502 | 504),
        "cancelled" => code == 499,
        _ => false,
    }
}

fn operation_matches(value: &Value, request: &Value, route_operation: Option<&str>) -> bool {
    let expected = route_operation.or_else(|| request["operation_id"].as_str());
    expected.is_some_and(|operation_id| value["operation_id"].as_str() == Some(operation_id))
}

fn action_matches_request(value: &Value, request: &Value) -> bool {
    request.is_null() || value["action"] == request["action"]
}

fn generation_shape(value: &Value, request: &Value, status: &str) -> bool {
    let Some(generation) = value["generation"].as_u64() else {
        return false;
    };
    if generation > MAX_GENERATION {
        return false;
    }
    if status == "settled" || request.is_null() {
        return true;
    }
    value["generation"] == request["generation"]
}

fn accepted(value: &Value) -> bool {
    value["action"].is_object()
        && value["observation"].is_null()
        && value["transition"].is_null()
        && value["effect_witness"].is_null()
        && value["error_code"].is_null()
}

fn rejected_or_uncertain(value: &Value) -> bool {
    value["action"].is_object()
        && value["observation"].is_null()
        && value["transition"].is_null()
        && value["effect_witness"].is_null()
        && identity(&value["error_code"])
}

fn settled(
    value: &Value,
    request: &Value,
    admissions: &BTreeMap<String, SelectorAdmission>,
    observation_valid: impl Fn(&Value) -> bool,
) -> bool {
    let observation = &value["observation"];
    let transition = &value["transition"];
    if !observation.is_object()
        || !observation_valid(observation)
        || observation["state_id"] != value["state_id"]
        || observation["generation"] != value["generation"]
        || !transition.is_object()
        || !transition_generations(transition, value, request)
        || !action_matches_transition(value, transition)
    {
        return false;
    }
    match transition["kind"].as_str() {
        Some("rest_option_selection_requested" | "rest_option_selection_progressed") => {
            transition["effect_witness"].is_null()
                && value["effect_witness"].is_null()
                && selector_valid(value, transition, admissions, request.is_null())
        }
        Some("rest_option_selection_completed") => {
            completed_selection_valid(value, transition, admissions)
        }
        Some("rest_option_completed") => completed_option_valid(value, transition),
        _ => false,
    }
}

fn transition_generations(transition: &Value, value: &Value, request: &Value) -> bool {
    let (Some(before), Some(after), Some(generation)) = (
        transition["before_generation"].as_u64(),
        transition["after_generation"].as_u64(),
        value["generation"].as_u64(),
    ) else {
        return false;
    };
    if before >= after || after != generation {
        return false;
    }
    request.is_null() || request["generation"] == before
}

fn action_matches_transition(value: &Value, transition: &Value) -> bool {
    let Some(payload) = value["action"]["action"].as_object() else {
        return false;
    };
    let Some(action_kind) = payload.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let Some(option_id) = transition["rest_option_id"].as_str() else {
        return false;
    };
    if payload.get("rest_option_id").and_then(Value::as_str) != Some(option_id) {
        return false;
    }
    match transition["kind"].as_str() {
        Some("rest_option_completed" | "rest_option_selection_requested") => {
            action_kind == "rest_option"
        }
        Some("rest_option_selection_progressed") => {
            let expected = match transition["selection_kind"].as_str() {
                Some("card") => "select_card",
                Some("player") => "select_player",
                _ => return false,
            };
            action_kind == expected
                && payload.get("selection_id") == transition.get("selection_id")
                && transition["selected_choice_ids"]
                    .as_array()
                    .and_then(|selected| selected.last())
                    .is_some_and(|choice| {
                        payload.get("card_id").or_else(|| payload.get("player_id")) == Some(choice)
                    })
        }
        Some("rest_option_selection_completed") => {
            if payload.get("selection_id") != transition.get("selection_id") {
                return false;
            }
            match (transition["selection_kind"].as_str(), action_kind) {
                (_, "confirm_selection") => true,
                (Some("card"), "select_card") => {
                    payload.get("card_id")
                        == transition["selected_choice_ids"]
                            .as_array()
                            .and_then(|choices| choices.last())
                }
                (Some("player"), "select_player") => {
                    payload.get("player_id")
                        == transition["selected_choice_ids"]
                            .as_array()
                            .and_then(|choices| choices.last())
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn completed_option_valid(value: &Value, transition: &Value) -> bool {
    transition["completed"] == true && completed_effect_valid(value, transition)
}

fn completed_effect_valid(value: &Value, transition: &Value) -> bool {
    let Some(root) = value["effect_witness"].as_object() else {
        return false;
    };
    if transition["effect_witness"] != value["effect_witness"]
        || root["version"] != "rest-effect-witness-v1"
        || root["operation_id"] != value["operation_id"]
        || root["rest_option_id"] != transition["rest_option_id"]
        || root["generation"] != value["generation"]
    {
        return false;
    }
    let Some(option) = transition["rest_option_id"].as_str() else {
        return false;
    };
    let expected = match option {
        "clone" => "clone_applied",
        "cook" => "cook_applied",
        "dig" => "dig_applied",
        "hatch" => "hatch_applied",
        "heal" => "heal_applied",
        "kindle" => "kindle_applied",
        "lift" => "lift_applied",
        "smith" => "smith_applied",
        "mend" => "mend_applied",
        _ => return false,
    };
    if root["kind"] != expected || !evidence_has_effect(value, root, option) {
        return false;
    }
    match transition["kind"].as_str() {
        Some("rest_option_selection_completed") => match option {
            "smith" => root["evidence"]["upgraded_card_ids"] == transition["selected_choice_ids"],
            "mend" => {
                root["target_player_id"]
                    == transition["selected_choice_ids"]
                        .as_array()
                        .and_then(|ids| ids.first())
                        .cloned()
                        .unwrap_or(Value::Null)
            }
            _ => true,
        },
        _ => true,
    }
}

fn evidence_has_effect(
    value: &Value,
    witness: &serde_json::Map<String, Value>,
    option: &str,
) -> bool {
    let evidence = &witness["evidence"];
    match option {
        "heal" => hp_evidence_has_effect(evidence),
        "mend" => match evidence["kind"].as_str() {
            Some("hp_change") => hp_evidence_has_effect(evidence),
            Some("native_completion") => native_evidence_matches(value, evidence),
            _ => false,
        },
        "smith" => {
            evidence["kind"] == "card_change"
                && evidence["upgraded_card_ids"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty())
        }
        "clone" => {
            evidence["kind"] == "card_change"
                && evidence["added_card_ids"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty())
        }
        "cook" => {
            evidence["kind"] == "card_change"
                && evidence["removed_card_ids"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty())
        }
        "dig" | "hatch" => {
            evidence["kind"] == "relic_change"
                && evidence["added_relic_ids"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty())
        }
        "kindle" => native_evidence_matches(value, evidence),
        "lift" => match evidence["kind"].as_str() {
            Some("stat_change") => true,
            Some("native_completion") => native_evidence_matches(value, evidence),
            _ => false,
        },
        _ => false,
    }
}

fn hp_evidence_has_effect(evidence: &Value) -> bool {
    let (Some(before), Some(after), Some(max_before), Some(max_after)) = (
        evidence["hp_before"].as_u64(),
        evidence["hp_after"].as_u64(),
        evidence["max_hp_before"].as_u64(),
        evidence["max_hp_after"].as_u64(),
    ) else {
        return false;
    };
    evidence["kind"] == "hp_change" && before <= max_before && after <= max_after && after > before
}

fn native_evidence_matches(value: &Value, evidence: &Value) -> bool {
    evidence["kind"] == "native_completion"
        && identity(&evidence["completion_id"])
        && evidence["native_state_id"] == value["observation"]["state_id"]
}

fn identity(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        !value.is_empty()
            && value.len() <= 128
            && !value.contains("..")
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
            })
    })
}

#[cfg(test)]
#[path = "runtime_v4_expert_rest_action_semantics_native_completion_tests.rs"]
mod native_completion_tests;
