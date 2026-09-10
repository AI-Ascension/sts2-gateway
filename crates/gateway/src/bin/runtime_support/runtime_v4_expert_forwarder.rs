// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::runtime_v4_expert::RuntimeV4ExpertRoute;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v4-expert/schema.json"
));
const ACTION_SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v4-expert-action/schema.json"
));
const DIGEST: &str = "0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42";
const ACTION_DIGEST: &str = "393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929";
const ACTION_PROTOCOL: &str = "runtime-v4-expert-action";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeV4ExpertForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertForwardError {
    RequestBodyForbidden,
    RequestBodyRequired,
    RequestBodyOversized,
    RequestBodyMalformed,
    ResponseOversized,
    ResponseMalformed,
}

impl RuntimeV4ExpertForwarder {
    pub(crate) const fn new(max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
        }
    }

    pub(crate) fn validate_request(
        self,
        route: &RuntimeV4ExpertRoute,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<Value, RuntimeV4ExpertForwardError> {
        if body.len() > self.max_request_bytes {
            return Err(RuntimeV4ExpertForwardError::RequestBodyOversized);
        }
        if !route.is_dispatch() {
            return if body.is_empty() {
                Ok(Value::Null)
            } else {
                Err(RuntimeV4ExpertForwardError::RequestBodyForbidden)
            };
        }
        if body.is_empty() {
            return Err(RuntimeV4ExpertForwardError::RequestBodyRequired);
        }
        let value = strict_json(body).ok_or(RuntimeV4ExpertForwardError::RequestBodyMalformed)?;
        if !valid_action_request(&value, headers) {
            return Err(RuntimeV4ExpertForwardError::RequestBodyMalformed);
        }
        Ok(value)
    }

    pub(crate) fn validate_response(
        self,
        route: &RuntimeV4ExpertRoute,
        request: &Value,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<(), RuntimeV4ExpertForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV4ExpertForwardError::ResponseOversized);
        }
        if route.is_state() {
            let value =
                validate_observation(body).ok_or(RuntimeV4ExpertForwardError::ResponseMalformed)?;
            if value["protocol_version"].as_str() != Some("runtime-v4-expert") {
                return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
            }
            return Ok(());
        }
        let value = strict_json(body).ok_or(RuntimeV4ExpertForwardError::ResponseMalformed)?;
        if !valid_action_response(&value, route.operation_id(), request, headers) {
            return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
        }
        if value["status"] == "settled" {
            let observation = serde_json::to_vec(&value["observation"])
                .map_err(|_| RuntimeV4ExpertForwardError::ResponseMalformed)?;
            let observation = validate_observation(&observation)
                .ok_or(RuntimeV4ExpertForwardError::ResponseMalformed)?;
            if observation["state_id"] != value["state_id"]
                || observation["generation"] != value["generation"]
            {
                return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
            }
            let transition = value["transition"]
                .as_object()
                .ok_or(RuntimeV4ExpertForwardError::ResponseMalformed)?;
            if request["generation"].is_number()
                && transition["before_generation"] != request["generation"]
            {
                return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
            }
            if transition["after_generation"] != value["generation"]
                || transition["removed"] != true
                || transition["after_generation"].as_u64()
                    <= transition["before_generation"].as_u64()
            {
                return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
            }
            let potion_id = value["action"]["action"]["potion_id"].as_str();
            if transition["potion_id"].as_str() != potion_id {
                return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
            }
        }
        Ok(())
    }
}

fn strict_json(body: &[u8]) -> Option<Value> {
    let value = super::strict_json::parse(body).ok()?;
    value.is_object().then_some(value)
}

fn valid_action_request(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 18
        && value["protocol_version"] == ACTION_PROTOCOL
        && value["schema_digest"] == ACTION_DIGEST
        && value["profile"] == "expert-action"
        && value["kind"] == "action_request"
        && value["status"].is_null()
        && value["observation"].is_null()
        && value["transition"].is_null()
        && value["error_code"].is_null()
        && headers_match(value, headers)
        && value["operation_id"]
            .as_str()
            .is_some_and(safe_operation_id)
        && value["state_id"].as_str().is_some_and(safe_identity)
        && valid_action(&value["action"])
        && schema_valid(ACTION_SCHEMA, value)
}

fn valid_action_response(
    value: &Value,
    route_operation: Option<&str>,
    request: &Value,
    headers: &BTreeMap<String, String>,
) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let expected_operation = route_operation.or_else(|| request["operation_id"].as_str());
    let operation_matches =
        expected_operation.is_some_and(|expected| value["operation_id"].as_str() == Some(expected));
    let request_action_matches = request["action"].is_null()
        || value["action"].is_null()
        || value["action"] == request["action"];
    let status = value["status"].as_str();
    let shape_matches = match status {
        Some("accepted") => {
            value["action"].is_object()
                && value["observation"].is_null()
                && value["transition"].is_null()
                && value["error_code"].is_null()
        }
        Some("settled") => {
            value["action"].is_object()
                && value["observation"].is_object()
                && value["transition"].is_object()
                && value["error_code"].is_null()
        }
        Some("rejected" | "unknown" | "cancelled") => {
            value["observation"].is_null()
                && value["transition"].is_null()
                && value["error_code"].as_str().is_some_and(safe_identity)
        }
        _ => false,
    };
    object.len() == 18
        && value["protocol_version"] == ACTION_PROTOCOL
        && value["schema_digest"] == ACTION_DIGEST
        && value["profile"] == "expert-action"
        && value["kind"] == "action_response"
        && operation_matches
        && request_action_matches
        && headers_match(value, headers)
        && schema_valid(ACTION_SCHEMA, value)
        && shape_matches
        && (status != Some("settled") || value["action"]["action"]["kind"] == "use_potion")
}

fn valid_action(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 2
        && object["action_id"].as_str().is_some_and(safe_identity)
        && object["action"].as_object().is_some_and(|action| {
            action.len() == 3
                && action["kind"] == "use_potion"
                && action["potion_id"].as_str().is_some_and(safe_identity)
                && (action["target_id"].is_null()
                    || action["target_id"].as_str().is_some_and(safe_identity))
        })
}

fn headers_match(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ]
    .into_iter()
    .all(|(field, header)| value[field].as_str() == headers.get(header).map(String::as_str))
        && value["lease_epoch"].as_u64()
            == headers
                .get("x-sts2-lease-epoch")
                .and_then(|value| value.parse::<u64>().ok())
}

fn validate_observation(body: &[u8]) -> Option<Value> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()?;
    let value = super::strict_json::parse(body).ok()?;
    (value["schema_digest"].as_str() == Some(DIGEST) && validator.is_valid(&value)).then_some(value)
}

fn schema_valid(schema_text: &str, value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    let validator = VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(schema_text).ok()?;
        jsonschema::validator_for(&schema).ok()
    });
    validator
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn safe_operation_id(value: &str) -> bool {
    safe_identity(value) && !value.contains('/')
}

#[cfg(test)]
#[path = "runtime_v4_expert_forwarder_tests.rs"]
mod tests;
