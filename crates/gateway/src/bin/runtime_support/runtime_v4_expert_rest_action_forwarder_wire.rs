// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::super::runtime_v4_expert_rest_action_semantics::SelectorAdmission;

pub(super) fn object_json(body: &[u8]) -> Option<Value> {
    let value = super::super::strict_json::parse(body).ok()?;
    value.is_object().then_some(value)
}

pub(super) fn request_valid(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    value["protocol_version"] == super::PROTOCOL_VERSION
        && value["schema_digest"] == super::SCHEMA_DIGEST
        && value["profile"] == super::PROFILE
        && value["kind"] == "action_request"
        && value["status"].is_null()
        && value["observation"].is_null()
        && value["transition"].is_null()
        && value["effect_witness"].is_null()
        && value["error_code"].is_null()
        && headers_match(value, headers)
        && identity(&value["state_id"])
        && identity(&value["operation_id"])
        && valid_action(&value["action"])
        && schema_valid(value)
}

pub(super) fn response_identity_valid(
    value: &Value,
    route: &RuntimeV4ExpertRestActionRoute,
    request: &Value,
    headers: &BTreeMap<String, String>,
    expected_action: Option<&Value>,
) -> bool {
    value["protocol_version"] == super::PROTOCOL_VERSION
        && value["schema_digest"] == super::SCHEMA_DIGEST
        && value["profile"] == super::PROFILE
        && value["kind"] == "action_response"
        && headers_match(value, headers)
        && identity(&value["state_id"])
        && identity(&value["operation_id"])
        && route
            .operation_id()
            .or_else(|| request["operation_id"].as_str())
            .is_some_and(|operation_id| value["operation_id"].as_str() == Some(operation_id))
        && expected_action.is_some_and(|action| value["action"] == *action)
}

fn headers_match(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ]
    .into_iter()
    .all(|(field, header)| {
        let Some(header_value) = headers.get(header) else {
            return false;
        };
        identity_string(header_value, super::NATIVE_IDENTITY_BYTES)
            && value[field].as_str() == Some(header_value)
    }) && value["lease_epoch"].as_u64()
        == headers
            .get("x-sts2-lease-epoch")
            .filter(|epoch| identity_string(epoch, super::NATIVE_IDENTITY_BYTES))
            .and_then(|epoch| epoch.parse::<u64>().ok())
}

fn valid_action(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 2 && identity(&value["action_id"]) && valid_action_payload(&value["action"])
}

fn valid_action_payload(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(kind) = value["kind"].as_str() else {
        return false;
    };
    let fields: &[&str] = match kind {
        "rest_option" => &["kind", "rest_option_id"],
        "select_card" => &["kind", "selection_id", "rest_option_id", "card_id"],
        "select_player" => &["kind", "selection_id", "rest_option_id", "player_id"],
        "confirm_selection" | "cancel_selection" => &["kind", "selection_id", "rest_option_id"],
        _ => return false,
    };
    object.len() == fields.len()
        && fields.iter().all(|field| {
            value
                .get(*field)
                .is_some_and(|member| *field == "kind" || identity(member))
        })
}

pub(super) fn action_admitted(
    value: &Value,
    admissions: &BTreeMap<String, SelectorAdmission>,
) -> bool {
    let Some(payload) = value["action"]["action"].as_object() else {
        return false;
    };
    let Some(selection_id) = payload.get("selection_id").and_then(Value::as_str) else {
        return payload.get("kind").and_then(Value::as_str) == Some("rest_option");
    };
    let Some(admission) = admissions.get(selection_id) else {
        return false;
    };
    let Some(generation) = value["generation"].as_u64() else {
        return false;
    };
    if generation != admission.generation {
        return false;
    }
    let Some(action_id) = value["action"]["action_id"].as_str() else {
        return false;
    };
    admission
        .legal_actions
        .get(action_id)
        .is_some_and(|admitted| admitted == &Value::Object(payload.clone()))
}

pub(super) fn schema_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(super::SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn identity(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|item| identity_string(item, super::NATIVE_IDENTITY_BYTES))
}

fn identity_string(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}
