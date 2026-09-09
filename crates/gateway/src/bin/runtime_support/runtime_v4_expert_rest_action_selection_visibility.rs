// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

pub(super) fn action_id_matches_kind(value: &Value, kind: &str) -> bool {
    let Some(action_id) = value.as_str() else {
        return false;
    };
    let Some(prefix) = action_id.split(':').next() else {
        return false;
    };
    prefix == kind || prefix.replace('_', "-") == kind.replace('_', "-")
}

pub(super) fn visible_card(
    value: &Value,
    action: &Map<String, Value>,
    selector: &Map<String, Value>,
) -> bool {
    action.get("card_id").is_some_and(|id| {
        super::visible_choice_ids(value)
            .is_some_and(|choices| id.as_str().is_some_and(|id| choices.contains(id)))
            && !selector["selected_choice_ids"]
                .as_array()
                .is_some_and(|selected| selected.iter().any(|item| item == id))
    })
}

pub(super) fn visible_player(
    value: &Value,
    action: &Map<String, Value>,
    selector: &Map<String, Value>,
) -> bool {
    action.get("player_id").is_some_and(|id| {
        super::visible_choice_ids(value)
            .is_some_and(|choices| id.as_str().is_some_and(|id| choices.contains(id)))
            && !selector["selected_choice_ids"]
                .as_array()
                .is_some_and(|selected| selected.iter().any(|item| item == id))
    })
}

pub(super) fn selected_action_is_visible(value: &Value, transition: &Value) -> bool {
    let Some(selected) = transition["selected_choice_ids"]
        .as_array()
        .or_else(|| transition["selector"]["selected_choice_ids"].as_array())
    else {
        return false;
    };
    match transition["selection_kind"]
        .as_str()
        .or_else(|| transition["selector"]["selection_kind"].as_str())
    {
        Some("card" | "player") => super::visible_choice_ids(value).is_some_and(|choices| {
            selected.iter().all(|choice| {
                choice
                    .as_str()
                    .is_some_and(|choice| choices.contains(choice))
            })
        }),
        _ => false,
    }
}
