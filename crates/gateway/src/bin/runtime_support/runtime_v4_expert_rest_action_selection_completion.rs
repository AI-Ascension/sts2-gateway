// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use super::super::identity;
use super::SelectorAdmission;

pub(crate) fn completed_selection_valid(
    value: &Value,
    transition: &Value,
    admissions: &BTreeMap<String, SelectorAdmission>,
) -> bool {
    let Some(selected) = transition["selected_choice_ids"].as_array() else {
        return false;
    };
    transition["remaining_count"] == 0
        && transition["completed"] == true
        && transition["required_count"].as_u64() == Some(selected.len() as u64)
        && completion_matches_admission(transition, admissions)
        // A completed response may return to `rest`; the retained admission
        // catalog binds durable choices after the selector is gone.
        && completed_selection_ids_are_valid(transition, admissions)
        && super::super::completed_effect_valid(value, transition)
}

fn completion_matches_admission(
    transition: &Value,
    admissions: &BTreeMap<String, SelectorAdmission>,
) -> bool {
    let Some(selection_id) = transition["selection_id"].as_str() else {
        return false;
    };
    if !identity(&transition["selection_id"]) {
        return false;
    }
    let Some(admission) = admissions.get(selection_id) else {
        return false;
    };
    let Some(selected) = transition["selected_choice_ids"].as_array() else {
        return false;
    };
    let Some(selected_ids) = ids_set(selected) else {
        return false;
    };
    admission.option_id == transition["rest_option_id"].as_str().unwrap_or_default()
        && admission.selection_kind == transition["selection_kind"].as_str().unwrap_or_default()
        && admission.required_count == transition["required_count"].as_u64().unwrap_or_default()
        && admission.generation == transition["before_generation"].as_u64().unwrap_or(u64::MAX)
        && admission.selected_choice_ids.is_subset(&selected_ids)
        && selected_ids.is_subset(&admission.choice_ids)
}

fn completed_selection_ids_are_valid(
    transition: &Value,
    admissions: &BTreeMap<String, SelectorAdmission>,
) -> bool {
    let Some(selected) = transition["selected_choice_ids"].as_array() else {
        return false;
    };
    let expected_kind = match transition["rest_option_id"].as_str() {
        Some("smith") => "card",
        Some("mend") => "player",
        _ => return false,
    };
    if transition["selection_kind"] != expected_kind {
        return false;
    }
    let Some(selection_id) = transition["selection_id"].as_str() else {
        return false;
    };
    let Some(admission) = admissions.get(selection_id) else {
        return false;
    };
    let Some(selected_ids) = ids_set(selected) else {
        return false;
    };
    selected.iter().all(identity)
        && admission.selection_kind == expected_kind
        && selected_ids.is_subset(&admission.choice_ids)
}

pub(super) fn ids_set(values: &[Value]) -> Option<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for value in values {
        let id = value.as_str()?;
        if !identity(value) || !ids.insert(id.to_owned()) {
            return None;
        }
    }
    Some(ids)
}
