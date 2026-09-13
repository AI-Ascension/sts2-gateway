// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::game_information::GameInformationRoute;
use super::game_information_payload as payload;
use super::http::MAX_BODY_BYTES;

pub(crate) const SCHEMA_DIGEST: &str =
    "e5ba81b0520687cf59db6a94aea3b38606e86300f6eb2b0e858f55704e62f76c";
pub(crate) const MAX_RESPONSE_BYTES: usize = payload::MAX_MESSAGE_BYTES;
pub(crate) const MAX_REQUEST_BYTES: usize = MAX_BODY_BYTES;
const PROTOCOL_VERSION: &str = "game-information-query-v1";
const ARTIFACT: &str = "sts2-protocol/game-information-query-v1";
const SCHEMA_SOURCE: &str = "schemas/game-information-query-v1.schema.json";
const GENERATOR: &str = "hand-authored";
const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/schema.json"
));

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GameInformationForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
    content_manifest_id: String,
    run_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameInformationRequestError {
    Required,
    Oversized,
    Invalid,
    Scope,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameInformationResponseError {
    Oversized,
    Invalid,
}

#[derive(Debug, PartialEq)]
pub(crate) struct ValidatedGameInformationResponse {
    pub(crate) value: Value,
    pub(crate) producer_error: bool,
}

impl GameInformationForwarder {
    pub(crate) fn new(
        max_request_bytes: usize,
        max_response_bytes: usize,
        content_manifest_id: &str,
        run_id: &str,
    ) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
            content_manifest_id: content_manifest_id.to_owned(),
            run_id: run_id.to_owned(),
        }
    }

    pub(crate) fn validate_request(
        &self,
        route: GameInformationRoute,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<Value, GameInformationRequestError> {
        if body.is_empty() {
            return Err(GameInformationRequestError::Required);
        }
        if body.len() > self.max_request_bytes {
            return Err(GameInformationRequestError::Oversized);
        }
        if !route.is_query() {
            return Err(GameInformationRequestError::Invalid);
        }
        let value =
            super::strict_json::parse(body).map_err(|_| GameInformationRequestError::Invalid)?;
        if !base_valid(&value)
            || value.get("kind").and_then(Value::as_str) != Some("query_request")
            || !correlation_matches(&value, headers)
        {
            return Err(GameInformationRequestError::Invalid);
        }
        let query = value
            .get("query")
            .ok_or(GameInformationRequestError::Invalid)?;
        match payload::validate_query(
            route,
            query,
            headers,
            &self.content_manifest_id,
            &self.run_id,
        ) {
            payload::QueryValidation::Valid => Ok(value),
            payload::QueryValidation::Scope => Err(GameInformationRequestError::Scope),
            payload::QueryValidation::Limit => Err(GameInformationRequestError::Limit),
            payload::QueryValidation::Invalid => Err(GameInformationRequestError::Invalid),
        }
    }

    pub(crate) fn validate_response(
        &self,
        route: GameInformationRoute,
        request: Option<&Value>,
        headers: &BTreeMap<String, String>,
        status: u16,
        body: &[u8],
    ) -> Result<ValidatedGameInformationResponse, GameInformationResponseError> {
        if body.len() > self.max_response_bytes || body.len() > MAX_RESPONSE_BYTES {
            return Err(GameInformationResponseError::Oversized);
        }
        let value =
            super::strict_json::parse(body).map_err(|_| GameInformationResponseError::Invalid)?;
        if !base_valid(&value) || !correlation_matches(&value, headers) {
            return Err(GameInformationResponseError::Invalid);
        }
        if value.get("kind").and_then(Value::as_str) == Some("error_response") {
            if status / 100 == 2 || !error_response_valid(&value) {
                return Err(GameInformationResponseError::Invalid);
            }
            return Ok(ValidatedGameInformationResponse {
                value,
                producer_error: true,
            });
        }
        if status != 200 {
            return Err(GameInformationResponseError::Invalid);
        }
        let valid = match route {
            GameInformationRoute::Capabilities => {
                value.get("kind").and_then(Value::as_str) == Some("capabilities_response")
                    && payload::validate_capabilities(&value)
            }
            _ => {
                value.get("kind").and_then(Value::as_str) == Some("query_response")
                    && request.is_some_and(|request| {
                        payload::validate_query_response(
                            request,
                            &value,
                            &self.content_manifest_id,
                            &self.run_id,
                            headers,
                        )
                    })
            }
        };
        valid
            .then_some(ValidatedGameInformationResponse {
                value,
                producer_error: false,
            })
            .ok_or(GameInformationResponseError::Invalid)
    }
}

fn base_valid(value: &Value) -> bool {
    value.get("protocol_version").and_then(Value::as_str) == Some(PROTOCOL_VERSION)
        && value.get("schema_digest").and_then(Value::as_str) == Some(SCHEMA_DIGEST)
        && provenance_valid(value.get("provenance"))
        && schema_valid(value)
}

fn schema_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn provenance_valid(value: Option<&Value>) -> bool {
    let Some(provenance) = value.and_then(Value::as_object) else {
        return false;
    };
    provenance.len() == 3
        && provenance.get("artifact").and_then(Value::as_str) == Some(ARTIFACT)
        && provenance.get("source").and_then(Value::as_str) == Some(SCHEMA_SOURCE)
        && provenance.get("generator").and_then(Value::as_str) == Some(GENERATOR)
}

fn correlation_matches(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    value.get("correlation_id").and_then(Value::as_str)
        == headers.get("x-sts2-correlation-id").map(String::as_str)
}

fn error_response_valid(value: &Value) -> bool {
    value.get("query") == Some(&Value::Null)
        && value.get("result") == Some(&Value::Null)
        && value.get("capabilities") == Some(&Value::Null)
        && value
            .get("error")
            .and_then(Value::as_object)
            .is_some_and(|error| error.get("code").and_then(Value::as_str).is_some())
}

#[cfg(test)]
#[path = "game_information_forwarder_tests.rs"]
mod tests;
