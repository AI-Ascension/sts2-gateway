// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;

use super::game_information_payload::{
    MAX_CURSOR_BYTES, MAX_MESSAGE_BYTES, MAX_PAGE_BYTES, MAX_PAGE_ITEMS, MAX_TEXT_BYTES,
    instance_matches_headers, normalized_query,
};

pub(super) fn validate_query_response(
    request: &Value,
    response: &Value,
    content_manifest_id: &str,
    run_id: &str,
    headers: &BTreeMap<String, String>,
) -> bool {
    let Some(request_query) = request.get("query") else {
        return false;
    };
    if response.get("query") != Some(request_query) {
        return false;
    }
    let Some(result) = response.get("result").and_then(Value::as_object) else {
        return false;
    };
    if result.get("read_only") != Some(&Value::Bool(true)) {
        return false;
    }
    let Some(page) = result.get("page").and_then(Value::as_object) else {
        return false;
    };
    if page.get("limits") != request_query.get("limits") {
        return false;
    }
    let mode = request_query
        .get("binding")
        .and_then(|binding| binding.get("mode"))
        .and_then(Value::as_str);
    if !result_generation_valid(result, request_query, mode)
        || !page_bounds_valid(page, request_query)
        || !items_valid(
            page,
            request_query,
            mode,
            content_manifest_id,
            run_id,
            headers,
        )
        || !cursor_binding_valid(page, request_query)
        || !totals_valid(page)
    {
        return false;
    }
    true
}

pub(super) fn validate_capabilities(value: &Value) -> bool {
    let Some(capabilities) = value.get("capabilities").and_then(Value::as_object) else {
        return false;
    };
    let all_queries_advertised = capabilities
        .get("query_kinds")
        .and_then(Value::as_array)
        .is_some_and(|kinds| {
            ["availability", "detail", "get", "list", "search"]
                .iter()
                .all(|kind| kinds.iter().any(|value| value.as_str() == Some(*kind)))
        });
    let Some(limits) = capabilities.get("limits").and_then(Value::as_object) else {
        return false;
    };
    capabilities.get("profile").and_then(Value::as_str) == Some("game-information-query-v1")
        && all_queries_advertised
        && limits_within_gateway_budget(limits)
        && capabilities
            .get("max_message_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| bytes as usize <= MAX_MESSAGE_BYTES)
        && capabilities
            .get("max_cursor_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| bytes as usize <= MAX_CURSOR_BYTES)
        && capabilities
            .get("snapshot_policy")
            .and_then(Value::as_object)
            .is_some_and(|policy| policy.get("supports_live") == Some(&Value::Bool(true)))
}

fn limits_within_gateway_budget(limits: &serde_json::Map<String, Value>) -> bool {
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

fn result_generation_valid(
    result: &serde_json::Map<String, Value>,
    query: &Value,
    mode: Option<&str>,
) -> bool {
    match mode {
        Some("static") => {
            result.get("result_generation") == Some(&Value::Null)
                && result.get("parent_observation") == Some(&Value::Null)
        }
        Some("live") => {
            let generation = query
                .get("binding")
                .and_then(|binding| binding.get("snapshot_ref"))
                .and_then(|snapshot| snapshot.get("state_generation"));
            result.get("result_generation") == generation
                && result.get("parent_observation") == query.get("parent_observation")
        }
        _ => false,
    }
}

fn page_bounds_valid(page: &serde_json::Map<String, Value>, query: &Value) -> bool {
    let Some(limits) = query.get("limits").and_then(Value::as_object) else {
        return false;
    };
    let Some(items) = page.get("items").and_then(Value::as_array) else {
        return false;
    };
    let Some(accounting) = page.get("accounting").and_then(Value::as_object) else {
        return false;
    };
    let item_limit = limits.get("page_items").and_then(Value::as_u64);
    let page_limit = limits.get("page_bytes").and_then(Value::as_u64);
    let text_limit = limits.get("text_bytes").and_then(Value::as_u64);
    let final_page = page.get("final_page") == Some(&Value::Bool(true));
    let next_cursor = page.get("next_cursor");
    if final_page != next_cursor.is_none_or(Value::is_null) {
        return false;
    }
    let measured_page_bytes = serde_json::to_vec(page)
        .ok()
        .map_or(u64::MAX, |bytes| bytes.len() as u64);
    let items_fit = items.iter().all(|item| {
        serde_json::to_vec(item)
            .ok()
            .is_some_and(|bytes| bytes.len() as u64 <= page_limit.unwrap_or(0))
    });
    items.len() as u64 <= item_limit.unwrap_or(0)
        && items_fit
        && measured_page_bytes <= page_limit.unwrap_or(0)
        && accounting.get("item_count").and_then(Value::as_u64) == Some(items.len() as u64)
        && accounting
            .get("payload_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| bytes <= page_limit.unwrap_or(0))
        && accounting
            .get("text_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|bytes| bytes <= text_limit.unwrap_or(0))
}

fn items_valid(
    page: &serde_json::Map<String, Value>,
    query: &Value,
    mode: Option<&str>,
    content_manifest_id: &str,
    run_id: &str,
    headers: &BTreeMap<String, String>,
) -> bool {
    let Some(items) = page.get("items").and_then(Value::as_array) else {
        return false;
    };
    let mut text_bytes = 0_u64;
    for item in items {
        let Some(item) = item.as_object() else {
            return false;
        };
        let Some(definition) = item.get("definition_ref").and_then(Value::as_object) else {
            return false;
        };
        if definition
            .get("content_manifest_id")
            .and_then(Value::as_str)
            != Some(content_manifest_id)
            || definition.get("entity_kind") != query.get("entity_kind")
            || !item_instance_valid(item, query, mode, headers, run_id)
        {
            return false;
        }
        let Some(fields) = item.get("fields").and_then(Value::as_array) else {
            return false;
        };
        for field in fields {
            let Some(field) = field.as_object() else {
                return false;
            };
            if !field_availability_valid(field) {
                return false;
            }
            text_bytes = text_bytes.saturating_add(field_text_bytes(field));
        }
    }
    page.get("accounting")
        .and_then(|accounting| accounting.get("text_bytes"))
        .and_then(Value::as_u64)
        .is_some_and(|declared| declared >= text_bytes)
}

fn item_instance_valid(
    item: &serde_json::Map<String, Value>,
    query: &Value,
    mode: Option<&str>,
    headers: &BTreeMap<String, String>,
    run_id: &str,
) -> bool {
    match mode {
        Some("static") => item.get("instance_ref") == Some(&Value::Null),
        Some("live") => item
            .get("instance_ref")
            .and_then(Value::as_object)
            .is_some_and(|instance| {
                let binding = query
                    .get("binding")
                    .and_then(|value| value.get("instance_ref"));
                Some(&Value::Object(instance.clone())) == binding
                    && instance_matches_headers(instance, headers, run_id)
            }),
        _ => false,
    }
}

fn field_availability_valid(field: &serde_json::Map<String, Value>) -> bool {
    let Some(availability) = field.get("availability").and_then(Value::as_str) else {
        return false;
    };
    let available = availability == "available";
    let value_present = !field.get("value").is_none_or(Value::is_null);
    let reason_present = !field.get("reason").is_none_or(Value::is_null);
    value_present == available && reason_present == !available
}

fn field_text_bytes(field: &serde_json::Map<String, Value>) -> u64 {
    match field.get("value") {
        Some(Value::String(value)) => value.len() as u64,
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(|value| value.len() as u64)
            .sum(),
        Some(Value::Object(_))
        | Some(Value::Bool(_))
        | Some(Value::Number(_))
        | Some(Value::Null)
        | None => field
            .get("reason")
            .and_then(Value::as_str)
            .map_or(0, |reason| reason.len() as u64),
    }
}

fn cursor_binding_valid(page: &serde_json::Map<String, Value>, query: &Value) -> bool {
    let next = page.get("next_cursor").and_then(Value::as_str);
    let binding = page.get("cursor_binding");
    if next.is_some() && binding.is_none_or(Value::is_null) {
        return false;
    }
    match binding {
        Some(Value::Null) | None => true,
        Some(binding) => normalized_query(query).is_some_and(|normalized| binding == &normalized),
    }
}

fn totals_valid(page: &serde_json::Map<String, Value>) -> bool {
    let known = page.get("total_count_known") == Some(&Value::Bool(true));
    let total = page.get("total_count");
    known == !total.is_none_or(Value::is_null)
}
