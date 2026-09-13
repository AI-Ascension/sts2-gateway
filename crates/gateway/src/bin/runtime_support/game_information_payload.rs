// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;

use super::game_information::GameInformationRoute;
pub(super) use super::game_information_payload_response::{
    validate_capabilities, validate_query_response,
};

pub(super) const MAX_PAGE_ITEMS: u64 = 32;
pub(super) const MAX_PAGE_BYTES: u64 = 65_536;
pub(super) const MAX_TEXT_BYTES: u64 = 4_096;
pub(super) const MAX_MESSAGE_BYTES: usize = 262_144;
pub(super) const MAX_CURSOR_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QueryValidation {
    Valid,
    Scope,
    Limit,
    Invalid,
}

pub(super) fn validate_query(
    route: GameInformationRoute,
    query: &Value,
    headers: &BTreeMap<String, String>,
    content_manifest_id: &str,
    run_id: &str,
) -> QueryValidation {
    let Some(kind) = route.query_kind() else {
        return QueryValidation::Invalid;
    };
    if query.get("query_kind").and_then(Value::as_str) != Some(kind) {
        return QueryValidation::Invalid;
    }
    let Some(binding) = query.get("binding").and_then(Value::as_object) else {
        return QueryValidation::Invalid;
    };
    if binding.get("content_manifest_id").and_then(Value::as_str) != Some(content_manifest_id) {
        return QueryValidation::Scope;
    }
    let Some(mode) = binding.get("mode").and_then(Value::as_str) else {
        return QueryValidation::Invalid;
    };
    if !limits_within_gateway_budget(query.get("limits").and_then(Value::as_object)) {
        return QueryValidation::Limit;
    }
    if !target_matches_content(query.get("target"), content_manifest_id)
        || !filters_valid(query.get("filters"), mode, headers, content_manifest_id)
    {
        return QueryValidation::Scope;
    }
    match mode {
        "static" => validate_static_query(query, binding),
        "live" => validate_live_query(query, binding, headers, run_id),
        _ => QueryValidation::Invalid,
    }
}

pub(super) fn normalized_query(query: &Value) -> Option<Value> {
    let mut normalized = query.clone();
    normalized.as_object_mut()?.remove("cursor");
    Some(normalized)
}

pub(super) fn cursor(query: &Value) -> Option<&str> {
    query.get("cursor").and_then(Value::as_str)
}

fn validate_static_query(
    query: &Value,
    binding: &serde_json::Map<String, Value>,
) -> QueryValidation {
    if binding.get("visibility_scope").and_then(Value::as_str) != Some("public")
        || binding.get("instance_ref") != Some(&Value::Null)
        || binding.get("snapshot_ref") != Some(&Value::Null)
        || query.get("parent_observation") != Some(&Value::Null)
    {
        return QueryValidation::Scope;
    }
    let target = query.get("target").and_then(Value::as_object);
    if target.and_then(|target| target.get("instance_ref")) != Some(&Value::Null) {
        return QueryValidation::Scope;
    }
    QueryValidation::Valid
}

fn validate_live_query(
    query: &Value,
    binding: &serde_json::Map<String, Value>,
    headers: &BTreeMap<String, String>,
    run_id: &str,
) -> QueryValidation {
    if binding.get("visibility_scope").and_then(Value::as_str) != Some("player") {
        return QueryValidation::Scope;
    }
    let Some(instance_ref) = binding.get("instance_ref").and_then(Value::as_object) else {
        return QueryValidation::Invalid;
    };
    let Some(snapshot_ref) = binding.get("snapshot_ref").and_then(Value::as_object) else {
        return QueryValidation::Invalid;
    };
    if !instance_matches_headers(instance_ref, headers, run_id)
        || snapshot_ref.get("instance_ref") != Some(&Value::Object(instance_ref.clone()))
        || !snapshot_generation_matches(snapshot_ref, query)
        || !parent_matches_snapshot(query, instance_ref, snapshot_ref)
    {
        return QueryValidation::Scope;
    }
    if query
        .get("target")
        .and_then(|target| target.get("instance_ref"))
        .is_some_and(|target| target != &Value::Object(instance_ref.clone()))
    {
        return QueryValidation::Scope;
    }
    QueryValidation::Valid
}

fn limits_within_gateway_budget(limits: Option<&serde_json::Map<String, Value>>) -> bool {
    let Some(limits) = limits else {
        return false;
    };
    limits
        .get("page_items")
        .and_then(Value::as_u64)
        .is_some_and(|value| (1..=MAX_PAGE_ITEMS).contains(&value))
        && limits
            .get("page_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|value| (1..=MAX_PAGE_BYTES).contains(&value))
        && limits
            .get("text_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|value| (1..=MAX_TEXT_BYTES).contains(&value))
}

fn target_matches_content(target: Option<&Value>, content_manifest_id: &str) -> bool {
    let Some(target) = target.and_then(Value::as_object) else {
        return false;
    };
    target
        .get("definition_ref")
        .and_then(Value::as_object)
        .is_none_or(|reference| {
            reference.get("content_manifest_id").and_then(Value::as_str)
                == Some(content_manifest_id)
        })
}

fn filters_valid(
    filters: Option<&Value>,
    mode: &str,
    headers: &BTreeMap<String, String>,
    content_manifest_id: &str,
) -> bool {
    let Some(filters) = filters.and_then(Value::as_object) else {
        return false;
    };
    let definitions = filters
        .get("definition_refs")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().all(|item| {
                item.get("content_manifest_id").and_then(Value::as_str) == Some(content_manifest_id)
            })
        });
    let instance_valid = match (mode, filters.get("instance_ids").and_then(Value::as_array)) {
        ("static", Some(items)) => items.is_empty(),
        ("live", Some(items)) => {
            let expected = headers.get("x-sts2-instance-id").map(String::as_str);
            items
                .iter()
                .all(|item| item.as_str().is_some_and(|id| Some(id) == expected))
        }
        _ => false,
    };
    definitions && instance_valid
}

pub(super) fn instance_matches_headers(
    instance: &serde_json::Map<String, Value>,
    headers: &BTreeMap<String, String>,
    run_id: &str,
) -> bool {
    let epoch = headers
        .get("x-sts2-lease-epoch")
        .and_then(|value| value.parse::<u64>().ok());
    instance.get("instance_id").and_then(Value::as_str)
        == headers.get("x-sts2-instance-id").map(String::as_str)
        && instance.get("run_id").and_then(Value::as_str) == Some(run_id)
        && instance.get("epoch").and_then(Value::as_u64) == epoch
}

fn snapshot_generation_matches(snapshot: &serde_json::Map<String, Value>, query: &Value) -> bool {
    let generation = snapshot.get("state_generation").and_then(Value::as_u64);
    query
        .get("parent_observation")
        .and_then(|parent| parent.get("state_generation"))
        .and_then(Value::as_u64)
        .is_none_or(|parent_generation| Some(parent_generation) == generation)
}

fn parent_matches_snapshot(
    query: &Value,
    instance_ref: &serde_json::Map<String, Value>,
    snapshot_ref: &serde_json::Map<String, Value>,
) -> bool {
    let Some(parent) = query.get("parent_observation") else {
        return false;
    };
    if parent.is_null() {
        return true;
    }
    parent.get("instance_ref") == Some(&Value::Object(instance_ref.clone()))
        && parent.get("snapshot_ref") == Some(&Value::Object(snapshot_ref.clone()))
        && parent.get("state_generation") == snapshot_ref.get("state_generation")
}
