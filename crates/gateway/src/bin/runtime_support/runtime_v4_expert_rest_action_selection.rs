// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use super::identity;

#[path = "runtime_v4_expert_rest_action_selection_completion.rs"]
mod completion;
pub(super) use completion::completed_selection_valid;
use completion::ids_set;
#[path = "runtime_v4_expert_rest_action_selection_visibility.rs"]
mod visibility;
use visibility::{selected_action_is_visible, visible_card, visible_player};

#[path = "runtime_v4_expert_rest_action_selection_admission.rs"]
mod admission;
pub(crate) use admission::{SelectorAdmission, SelectorLifecycle, admission_from_transition};

fn selection_choice_id(legal: &Value) -> Option<&str> {
    let action = legal["action"].as_object()?;
    match action.get("kind").and_then(Value::as_str) {
        Some("select_card") => action.get("card_id")?.as_str(),
        Some("select_player") => action.get("player_id")?.as_str(),
        _ => None,
    }
}

pub(super) fn selector_valid(
    value: &Value,
    transition: &Value,
    admissions: &std::collections::BTreeMap<String, SelectorAdmission>,
    reconciling: bool,
) -> bool {
    let Some(selector) = transition["selector"].as_object() else {
        return false;
    };
    let Some(lifecycle) = SelectorLifecycle::from_value(value) else {
        return false;
    };
    if selector.len() != 6 {
        return false;
    }
    if transition["kind"] == "rest_option_selection_progressed"
        && ([
            "selection_id",
            "selection_kind",
            "required_count",
            "selected_choice_ids",
            "remaining_count",
        ]
        .into_iter()
        .any(|field| selector[field] != transition[field]))
    {
        return false;
    }
    let option = transition["rest_option_id"].as_str();
    let expected_kind = match option {
        Some("smith") => "card",
        Some("mend") => "player",
        _ => return false,
    };
    if selector["selection_kind"].as_str() != Some(expected_kind) {
        return false;
    }
    let Some(required) = selector["required_count"].as_u64() else {
        return false;
    };
    let Some(selected) = selector["selected_choice_ids"].as_array() else {
        return false;
    };
    let Some(remaining) = selector["remaining_count"].as_u64() else {
        return false;
    };
    let Some(selection_id) = selector["selection_id"].as_str() else {
        return false;
    };
    if !identity(&selector["selection_id"])
        || !identity(&transition["rest_option_id"])
        || (transition["kind"] == "rest_option_selection_progressed"
            && !identity(&transition["selection_id"]))
    {
        return false;
    }
    if selected.len() as u64 > required || remaining != required - selected.len() as u64 {
        return false;
    }
    let Some(selected_ids) = ids_set(selected) else {
        return false;
    };
    let Some(visible_ids) = visible_choice_ids(value) else {
        return false;
    };
    if !selected_ids.is_subset(&visible_ids) {
        return false;
    }
    if let Some(admission) = admissions.get(selection_id) {
        if admission.lifecycle != lifecycle
            || option != Some(admission.option_id.as_str())
            || admission.selection_kind != expected_kind
            || admission.required_count != required
            || !selector_admission_generation_and_progress(
                admission,
                transition,
                &visible_ids,
                &selected_ids,
                reconciling,
            )
        {
            return false;
        }
    } else if transition["kind"] != "rest_option_selection_requested" {
        // Progress and completion require an observed selector catalog.
        return false;
    }
    let Some(legal_actions) = selector["legal_actions"].as_array() else {
        return false;
    };
    let mut action_ids = BTreeSet::new();
    let mut current_legal_actions = BTreeMap::new();
    let mut legal_choice_ids = BTreeSet::new();
    let mut has_confirm = false;
    for legal in legal_actions {
        let Some(action_id) = legal["action_id"].as_str() else {
            return false;
        };
        if !valid_selection_action(value, legal, selector, transition)
            || !action_ids.insert(action_id)
            || current_legal_actions
                .insert(action_id.to_owned(), legal["action"].clone())
                .is_some()
        {
            return false;
        }
        if let Some(choice) = selection_choice_id(legal)
            && (!visible_ids.contains(choice)
                || selected_ids.contains(choice)
                || !legal_choice_ids.insert(choice.to_owned()))
        {
            return false;
        }
        if let Some(admission) = admissions.get(selection_id)
            && let Some(choice) = selection_choice_id(legal)
            && !admission.choice_ids.contains(choice)
        {
            return false;
        }
        has_confirm |= legal["action"]["kind"] == "confirm_selection";
    }
    if admissions.get(selection_id).is_some_and(|admission| {
        admission.generation == transition["after_generation"].as_u64().unwrap_or(u64::MAX)
            && admission.legal_actions != current_legal_actions
    }) {
        // A reconciliation at an already admitted generation is a replay of
        // the same catalog. The producer may rotate opaque action IDs while a
        // selection advances to a new generation, but it cannot rewrite an
        // already observed catalog at the same generation.
        return false;
    }
    let has_select = legal_actions.iter().any(|legal| {
        matches!(
            legal["action"]["kind"].as_str(),
            Some("select_card" | "select_player")
        )
    });
    let has_cancel = legal_actions
        .iter()
        .any(|legal| legal["action"]["kind"] == "cancel_selection");
    let mut catalog_ids = selected_ids;
    catalog_ids.extend(legal_choice_ids);
    if catalog_ids != visible_ids {
        // Visible choices must be selected or selectable legal actions.
        return false;
    }
    (remaining == 0) == has_confirm
        && (remaining > 0) == has_select
        && has_cancel
        && selected_action_is_visible(value, transition)
}

fn selector_admission_generation_and_progress(
    admission: &SelectorAdmission,
    transition: &Value,
    visible_ids: &BTreeSet<String>,
    selected_ids: &BTreeSet<String>,
    reconciling: bool,
) -> bool {
    let kind = transition["kind"].as_str();
    if admission.choice_ids != *visible_ids {
        return false;
    }
    if reconciling {
        let at_progress_after = admission.generation
            == transition["after_generation"].as_u64().unwrap_or(u64::MAX)
            && admission.selected_choice_ids == *selected_ids;
        let at_progress_before = admission.generation
            == transition["before_generation"].as_u64().unwrap_or(u64::MAX)
            && admission.selected_choice_ids.is_subset(selected_ids)
            && selected_ids.len() == admission.selected_choice_ids.len() + 1;
        return match kind {
            Some("rest_option_selection_progressed") => at_progress_before || at_progress_after,
            _ => at_progress_after,
        };
    }
    match kind {
        Some("rest_option_selection_requested") => {
            admission.generation == transition["after_generation"].as_u64().unwrap_or(u64::MAX)
                && admission.selected_choice_ids == *selected_ids
        }
        Some("rest_option_selection_progressed") => {
            admission.generation == transition["before_generation"].as_u64().unwrap_or(u64::MAX)
                && admission.selected_choice_ids.is_subset(selected_ids)
                && selected_ids.len() == admission.selected_choice_ids.len() + 1
        }
        _ => false,
    }
}

fn valid_selection_action(
    value: &Value,
    legal: &Value,
    selector: &serde_json::Map<String, Value>,
    transition: &Value,
) -> bool {
    let Some(action) = legal["action"].as_object() else {
        return false;
    };
    let Some(kind) = action.get("kind").and_then(Value::as_str) else {
        return false;
    };
    if !identity(&legal["action_id"])
        || !identity(action.get("selection_id").unwrap_or(&Value::Null))
        || !identity(action.get("rest_option_id").unwrap_or(&Value::Null))
        || action.get("selection_id") != selector.get("selection_id")
        || action.get("rest_option_id") != transition.get("rest_option_id")
    {
        return false;
    }
    match kind {
        "confirm_selection" | "cancel_selection" => action.len() == 3,
        "select_card" => {
            selector["selection_kind"] == "card"
                && action.len() == 4
                && identity(action.get("card_id").unwrap_or(&Value::Null))
                && visible_card(value, action, selector)
        }
        "select_player" => {
            selector["selection_kind"] == "player"
                && action.len() == 4
                && identity(action.get("player_id").unwrap_or(&Value::Null))
                && visible_player(value, action, selector)
        }
        _ => false,
    }
}

fn visible_choice_ids(value: &Value) -> Option<BTreeSet<String>> {
    let choices = value["observation"]["state"]["choices"].as_array()?;
    let mut ids = BTreeSet::new();
    for choice in choices {
        let id = choice["choice_id"].as_str()?;
        if !identity(&choice["choice_id"]) || !ids.insert(id.to_owned()) {
            return None;
        }
    }
    Some(ids)
}
