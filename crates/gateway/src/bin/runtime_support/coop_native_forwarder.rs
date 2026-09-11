// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::coop_native::CoopNativeRoute;

#[path = "coop_native_forwarder_relations.rs"]
mod relations;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-native-v1/schema.json"
));
pub(crate) const COOP_NATIVE_SCHEMA_DIGEST: &str =
    "2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629";
const PROTOCOL_VERSION: &str = "coop-native-v1";
const ARTIFACT: &str = "sts2-protocol/coop-native-v1";
const SCHEMA_SOURCE: &str = "schemas/coop-native-v1.schema.json";
const GENERATOR: &str = "hand-authored";
const MAX_ERROR_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoopNativeForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoopNativeForwardError {
    RequestBodyRequired,
    RequestBodyForbidden,
    RequestBodyOversized,
    RequestBodyMalformed,
    ResponseOversized,
    ResponseMalformed,
}

impl CoopNativeForwarder {
    pub(crate) const fn new(max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
        }
    }

    pub(crate) fn validate_request(
        self,
        route: CoopNativeRoute,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<Option<Value>, CoopNativeForwardError> {
        if route == CoopNativeRoute::Observation {
            return body
                .is_empty()
                .then_some(None)
                .ok_or(CoopNativeForwardError::RequestBodyForbidden);
        }
        if body.is_empty() {
            return Err(CoopNativeForwardError::RequestBodyRequired);
        }
        if body.len() > self.max_request_bytes {
            return Err(CoopNativeForwardError::RequestBodyOversized);
        }
        let value = parse_and_validate(body).ok_or(CoopNativeForwardError::RequestBodyMalformed)?;
        if value["kind"].as_str() != route.request_kind()
            || !headers_match(&value, headers)
            || !relations::request_relations(route, &value)
        {
            return Err(CoopNativeForwardError::RequestBodyMalformed);
        }
        Ok(Some(value))
    }

    pub(crate) fn validate_response(
        self,
        route: CoopNativeRoute,
        request: Option<&Value>,
        headers: &BTreeMap<String, String>,
        status: u16,
        body: &[u8],
    ) -> Result<Value, CoopNativeForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(CoopNativeForwardError::ResponseOversized);
        }
        let value = parse_and_validate(body).ok_or(CoopNativeForwardError::ResponseMalformed)?;
        if value["kind"].as_str() != Some(route.response_kind())
            || !headers_match(&value, headers)
            || !relations::response_relations(route, request, status, &value)
        {
            return Err(CoopNativeForwardError::ResponseMalformed);
        }
        Ok(value)
    }

    /// Native host errors are deliberately kept outside the protocol envelope.  Only the
    /// producer's bounded `{error_code}` shape and statuses are allowed through this escape hatch;
    /// arbitrary downstream JSON never becomes a gateway response.
    pub(crate) fn is_bounded_error(status: u16, body: &[u8]) -> bool {
        if !matches!(status, 400 | 409 | 503) || body.is_empty() || body.len() > MAX_ERROR_BYTES {
            return false;
        }
        let Ok(value) = super::strict_json::parse(body) else {
            return false;
        };
        let Some(object) = value.as_object() else {
            return false;
        };
        object.len() == 1 && value["error_code"].as_str().is_some_and(valid_error_code)
    }
}

fn parse_and_validate(body: &[u8]) -> Option<Value> {
    let value = super::strict_json::parse(body).ok()?;
    if value["protocol_version"] != PROTOCOL_VERSION
        || value["schema_digest"] != COOP_NATIVE_SCHEMA_DIGEST
        || !provenance_valid(&value)
        || !schema_valid(&value)
    {
        return None;
    }
    Some(value)
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

fn provenance_valid(value: &Value) -> bool {
    let Some(provenance) = value["provenance"].as_object() else {
        return false;
    };
    provenance.len() == 3
        && provenance["artifact"] == ARTIFACT
        && provenance["source"] == SCHEMA_SOURCE
        && provenance["generator"] == GENERATOR
}

fn headers_match(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    [
        ("correlation_id", "x-sts2-correlation-id"),
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
    ]
    .into_iter()
    .all(|(field, header)| value[field].as_str() == headers.get(header).map(String::as_str))
        && value["lease_epoch"].as_u64()
            == headers
                .get("x-sts2-lease-epoch")
                .and_then(|epoch| epoch.parse::<u64>().ok())
}

fn valid_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
#[path = "coop_native_forwarder_tests.rs"]
mod tests;
