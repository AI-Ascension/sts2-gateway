// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectorLifecycle {
    pub(crate) instance_id: String,
    pub(crate) session_id: String,
    pub(crate) lease_id: String,
    pub(crate) lease_epoch: u64,
}

impl SelectorLifecycle {
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            instance_id: value["instance_id"].as_str()?.to_owned(),
            session_id: value["session_id"].as_str()?.to_owned(),
            lease_id: value["lease_id"].as_str()?.to_owned(),
            lease_epoch: value["lease_epoch"].as_u64()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectorAdmission {
    pub(super) option_id: String,
    pub(super) selection_kind: String,
    pub(super) required_count: u64,
    pub(crate) generation: u64,
    pub(crate) lifecycle: SelectorLifecycle,
    pub(super) choice_ids: BTreeSet<String>,
    pub(crate) selected_choice_ids: BTreeSet<String>,
    pub(crate) legal_actions: BTreeMap<String, Value>,
}

pub(crate) fn admission_from_transition(
    transition: &Value,
    lifecycle: SelectorLifecycle,
) -> Option<(String, SelectorAdmission)> {
    let selector = transition["selector"].as_object()?;
    let selection_id = selector["selection_id"].as_str()?.to_owned();
    let option_id = transition["rest_option_id"].as_str()?.to_owned();
    let selection_kind = selector["selection_kind"].as_str()?.to_owned();
    let required_count = selector["required_count"].as_u64()?;
    let generation = transition["after_generation"].as_u64()?;
    let mut choice_ids = BTreeSet::new();
    let mut selected_choice_ids = BTreeSet::new();
    for choice in selector["selected_choice_ids"].as_array()? {
        let choice = choice.as_str()?.to_owned();
        if !selected_choice_ids.insert(choice.clone()) {
            return None;
        }
        choice_ids.insert(choice);
    }
    let mut legal_actions = BTreeMap::new();
    for legal in selector["legal_actions"].as_array()? {
        let action_id = legal["action_id"].as_str()?.to_owned();
        let action = legal["action"].clone();
        if legal_actions.insert(action_id, action.clone()).is_some() {
            return None;
        }
        if let Some(choice) = selection_choice_id(legal) {
            choice_ids.insert(choice.to_owned());
        }
    }
    Some((
        selection_id,
        SelectorAdmission {
            option_id,
            selection_kind,
            required_count,
            generation,
            lifecycle,
            choice_ids,
            selected_choice_ids,
            legal_actions,
        },
    ))
}

fn selection_choice_id(legal: &Value) -> Option<&str> {
    let action = legal["action"].as_object()?;
    match action.get("kind").and_then(Value::as_str) {
        Some("select_card") => action.get("card_id")?.as_str(),
        Some("select_player") => action.get("player_id")?.as_str(),
        _ => None,
    }
}
