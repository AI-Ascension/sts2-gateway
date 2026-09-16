// SPDX-License-Identifier: MIT

//! Closed, Gateway-owned offer projection for negotiated capability snapshots.

use super::*;

pub(super) fn known_runtime_v3_baseline_offers(scopes: &[&str], scope_bits: u8) -> Vec<Value> {
    [
        ("state", "read", 0b001),
        ("legal_actions", "read", 0b001),
        ("dispatch_action", "mutate", 0b010),
        ("wait", "read", 0b001),
        ("reobserve", "read", 0b001),
        ("recover", "control", 0b100),
    ]
    .into_iter()
    .filter(|(_, _, required_bit)| scope_bits & required_bit != 0)
    .map(|(operation, required_scope, _)| {
        json!({
            "operation": format!("runtime_v3.{operation}"),
            "revision": RUNTIME_V3_BASELINE_PROFILE,
            "required_scope": required_scope,
            "scope": scopes,
            "wire_limits": {
                "max_request_bytes": super::MAX_BODY_BYTES,
                "max_response_bytes": super::MAX_RESPONSE_BYTES,
            },
            "content_limits": {
                "max_content_bytes": super::MAX_RESPONSE_BYTES,
                "max_page_items": 1,
            },
        })
    })
    .collect()
}

pub(super) fn known_lookup_binding_offers(scopes: &[&str]) -> [Value; 2] {
    ["discovery", "observe"].map(|operation| {
        json!({
            "operation": format!("game_information.lookup_binding.{operation}"),
            "revision": LOOKUP_BINDING_PROFILE,
            "required_scope": "read",
            "scope": scopes,
            "wire_limits": {
                "max_request_bytes": super::MAX_BODY_BYTES,
                "max_response_bytes": super::super::super::game_information_lookup_binding::RESPONSE_LIMIT_BYTES,
            },
            "content_limits": {
                "max_content_bytes": super::super::super::game_information_lookup_binding::RESPONSE_LIMIT_BYTES,
                "max_page_items": 1,
            },
        })
    })
}

pub(super) fn known_game_information_capabilities_offer(
    capabilities: &super::game_information_forwarder::GameInformationCapabilities,
    scopes: &[&str],
) -> Value {
    json!({
        "operation": "game_information.capabilities",
        "revision": GAME_INFORMATION_PROFILE,
        "required_scope": "read",
        "scope": scopes,
        "wire_limits": {
            // The producer endpoint is a bodyless GET, represented as zero bytes.
            "max_request_bytes": 0,
            "max_response_bytes": capabilities.max_message_bytes,
        },
        "content_limits": {
            "max_content_bytes": capabilities.max_message_bytes,
            "max_page_items": 1,
        },
    })
}

pub(super) fn known_game_information_offer(
    kind: &str,
    capabilities: &super::game_information_forwarder::GameInformationCapabilities,
    scopes: &[&str],
) -> Option<Value> {
    let operation = match kind {
        "list" => "game_information.list",
        "search" => "game_information.search",
        "get" => "game_information.get",
        "detail" => "game_information.detail",
        "availability" => "game_information.availability",
        _ => return None,
    };
    Some(json!({
        "operation": operation,
        "revision": GAME_INFORMATION_PROFILE,
        "required_scope": "read",
        "scope": scopes,
        "wire_limits": {
            "max_request_bytes": capabilities.max_message_bytes,
            "max_response_bytes": capabilities.max_message_bytes,
        },
        "content_limits": {
            "max_content_bytes": capabilities.limits.page_bytes,
            "max_page_items": capabilities.limits.page_items,
        },
    }))
}
