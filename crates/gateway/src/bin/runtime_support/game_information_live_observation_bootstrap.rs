// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Value, json};

use super::game_information_forwarder::GameInformationProducerAuthority;
use super::game_information_lookup_binding::BoundLookupBinding;
use super::http::{MAX_BODY_BYTES, MAX_RESPONSE_BYTES};
use super::strict_json;

pub(crate) const SCHEMA_DIGEST: &str =
    "6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2";
pub(crate) const PROFILE: &str = "game-information-live-observation-bootstrap-v1";
pub(crate) const MAX_REQUEST_BYTES: usize = MAX_BODY_BYTES;
pub(crate) const MAX_BOOTSTRAP_RESPONSE_BYTES: usize = MAX_RESPONSE_BYTES;
const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-live-observation-bootstrap-v1/schema.json"
));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[rustfmt::skip]
pub(crate) enum Error {
    Required, Oversized, Invalid, Scope,
}

pub(crate) fn validate_request(
    body: &[u8],
    headers: &BTreeMap<String, String>,
    authority: &GameInformationProducerAuthority,
    binding: Option<&BoundLookupBinding>,
    locale: &str,
) -> Result<Value, Error> {
    if body.is_empty() {
        return Err(Error::Required);
    }
    if body.len() > MAX_REQUEST_BYTES {
        return Err(Error::Oversized);
    }
    let value = strict_json::parse(body).map_err(|_| Error::Invalid)?;
    if !schema_valid(&value)
        || value.get("kind").and_then(Value::as_str) != Some("bootstrap_request")
        || !correlation_matches(&value, headers)
    {
        return Err(Error::Invalid);
    }
    if !scope_matches(
        value.get("scope"),
        authority,
        binding,
        locale,
        value.get("selector"),
    ) {
        return Err(Error::Scope);
    }
    if !request_limits_valid(&value) {
        return Err(Error::Invalid);
    }
    Ok(value)
}

pub(crate) fn validate_response(
    body: &[u8],
    request: &Value,
    headers: &BTreeMap<String, String>,
    authority: &GameInformationProducerAuthority,
    binding: Option<&BoundLookupBinding>,
    locale: &str,
    status: u16,
) -> Result<(Value, bool), Error> {
    if body.len() > MAX_BOOTSTRAP_RESPONSE_BYTES {
        return Err(Error::Oversized);
    }
    let value = strict_json::parse(body).map_err(|_| Error::Invalid)?;
    if !schema_valid(&value) || !correlation_matches(&value, headers) {
        return Err(Error::Invalid);
    }
    if value.get("scope") != request.get("scope")
        || value.get("selector") != request.get("selector")
        || !scope_matches(
            value.get("scope"),
            authority,
            binding,
            locale,
            value.get("selector"),
        )
    {
        return Err(Error::Scope);
    }
    match value.get("kind").and_then(Value::as_str) {
        Some("error_response") => {
            if !(400..=599).contains(&status) || !error_response_valid(&value) {
                return Err(Error::Invalid);
            }
            Ok((value, true))
        }
        Some("bootstrap_response") => {
            if status != 200
                || !limits_within(&value, request)
                || !request_limits_valid(&value)
                || !semantic_valid(&value)
            {
                return Err(Error::Invalid);
            }
            Ok((value, false))
        }
        _ => Err(Error::Invalid),
    }
}

fn schema_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            if sts2_gateway::sha256_hex(SCHEMA.as_bytes()) != SCHEMA_DIGEST {
                return None;
            }
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn correlation_matches(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    value
        .get("correlation_id")
        .and_then(Value::as_str)
        .zip(headers.get("x-sts2-correlation-id").map(String::as_str))
        .is_some_and(|(body, header)| body == header)
}

fn scope_matches(
    scope: Option<&Value>,
    authority: &GameInformationProducerAuthority,
    binding: Option<&BoundLookupBinding>,
    locale: &str,
    selector: Option<&Value>,
) -> bool {
    let Some(scope) = scope.and_then(Value::as_object) else {
        return false;
    };
    if scope.get("instance_id").and_then(Value::as_str) != Some(&authority.instance_id)
        || scope.get("run_id").and_then(Value::as_str) != Some(&authority.run_id)
        || scope.get("content_manifest_id").and_then(Value::as_str)
            != Some(&authority.content_manifest_id)
        || scope.get("locale").and_then(Value::as_str) != Some(locale)
    {
        return false;
    }
    let Some(epoch) = scope.get("authority_epoch").and_then(Value::as_u64) else {
        return false;
    };
    let Some(binding) = binding else {
        return false;
    };
    if binding.authority != *authority
        || binding.content_manifest_id != authority.content_manifest_id
        || binding.authority_epoch != epoch
        || binding.scope.run_id != authority.run_id
    {
        return false;
    }
    selector
        .and_then(|value| value.get("definition_ref"))
        .and_then(|value| value.get("content_manifest_id"))
        .and_then(Value::as_str)
        == Some(&authority.content_manifest_id)
}

fn request_limits_valid(value: &Value) -> bool {
    let Some(limits) = value.get("limits").and_then(Value::as_object) else {
        return false;
    };
    limits
        .get("max_visible_entities")
        .and_then(Value::as_u64)
        .is_some_and(|value| value <= 64)
        && limits
            .get("max_item_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= MAX_RESPONSE_BYTES as u64)
        && limits
            .get("max_message_bytes")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= MAX_RESPONSE_BYTES as u64)
}

fn limits_within(response: &Value, request: &Value) -> bool {
    [
        "max_visible_entities",
        "max_item_bytes",
        "max_message_bytes",
    ]
    .into_iter()
    .all(|field| {
        response["limits"][field].as_u64().is_some_and(|value| {
            request["limits"][field]
                .as_u64()
                .is_some_and(|limit| value <= limit)
        })
    })
}

fn semantic_valid(document: &Value) -> bool {
    let parent = &document["parent_observation"];
    let parent_ref = &parent["instance_ref"];
    parent_ref == &parent["snapshot_ref"]["instance_ref"]
        && parent["state_generation"] == parent["snapshot_ref"]["state_generation"]
        && document["scope"]["instance_id"] == parent_ref["instance_id"]
        && document["scope"]["run_id"] == parent_ref["run_id"]
        && document["scope"]["content_manifest_id"]
            == document["selector"]["definition_ref"]["content_manifest_id"]
        && owner_provenance_valid(&document["owner_provenance"])
        && encoded_bounds_valid(document)
        && visible_entities_valid(document)
}

fn encoded_bounds_valid(document: &Value) -> bool {
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

fn visible_entities_valid(document: &Value) -> bool {
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

fn owner_provenance_valid(value: &Value) -> bool {
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

fn error_response_valid(value: &Value) -> bool {
    value["parent_observation"].is_null()
        && value["visible_entities"].is_null()
        && value["error"]["code"].as_str().is_some_and(|code| {
            matches!(
                code,
                "not_observable" | "stale_snapshot" | "invalid_binding" | "invalid_bounds"
            )
        })
}
