// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

pub(super) fn encoded_bounds_valid(document: &Value) -> bool {
    let item_limit = document["limits"]["max_item_bytes"].as_u64().unwrap_or(0);
    let message_limit = document["limits"]["max_message_bytes"]
        .as_u64()
        .unwrap_or(0);
    document["visible_entities"]
        .as_array()
        .is_some_and(|entities| {
            entities.iter().all(|entity| {
                serde_json::to_vec(entity)
                    .ok()
                    .is_some_and(|bytes| bytes.len() as u64 <= item_limit)
            })
        })
        && serde_json::to_vec(document)
            .ok()
            .is_some_and(|bytes| bytes.len() as u64 <= message_limit)
}

pub(super) fn visible_entities_valid(document: &Value) -> bool {
    let Some(visible) = document["visible_entities"].as_array() else {
        return false;
    };
    if visible.len() as u64
        > document["limits"]["max_visible_entities"]
            .as_u64()
            .unwrap_or(0)
    {
        return false;
    }
    let parent = &document["parent_observation"];
    let parent_ref = &parent["instance_ref"];
    let definition = &document["selector"]["definition_ref"];
    let selector_ref = &document["selector"]["instance_ref"];
    let mut identities = Vec::with_capacity(visible.len());
    for entity in visible {
        let instance = &entity["instance_ref"];
        let snapshot = &entity["snapshot_ref"];
        if ["instance_id", "run_id", "epoch"]
            .into_iter()
            .any(|field| instance[field] != parent_ref[field])
            || snapshot["instance_ref"] != *instance
            || snapshot["snapshot_id"] != parent["snapshot_ref"]["snapshot_id"]
            || snapshot["state_generation"] != parent["state_generation"]
            || (entity["definition_ref"].is_object() && entity["definition_ref"] != *definition)
            || (entity["definition_ref"].is_object()
                && entity["definition_ref"]["content_manifest_id"]
                    != definition["content_manifest_id"])
        {
            return false;
        }
        let identity = json!([
            instance["instance_id"],
            instance["run_id"],
            instance["epoch"],
            instance["entity_kind"],
            instance["entity_id"]
        ]);
        if identities.iter().any(|seen| seen == &identity) {
            return false;
        }
        identities.push(identity);
    }
    visible
        .iter()
        .any(|entity| entity["instance_ref"] == *parent_ref)
        && (selector_ref.is_null()
            || (selector_ref == parent_ref
                && visible
                    .iter()
                    .filter(|entity| entity["instance_ref"] == *selector_ref)
                    .count()
                    == 1))
}

pub(super) fn owner_provenance_valid(value: &Value) -> bool {
    value
        == &json!({
            "authority_epoch_owner": "sts2-harness",
            "content_manifest_owner": "sts2-game-mod",
            "instance_fence_owner": "sts2-gateway",
            "instance_ref_epoch_owner": "sts2-game-mod",
            "native_snapshot_owner": "sts2-game-mod",
            "transport_lease_epoch_role": "fence_only"
        })
}

pub(super) fn error_response_valid(value: &Value) -> bool {
    value["parent_observation"].is_null()
        && value["visible_entities"].is_null()
        && value["error"]["code"].as_str().is_some_and(|code| {
            matches!(
                code,
                "not_observable" | "stale_snapshot" | "invalid_binding" | "invalid_bounds"
            )
        })
}
