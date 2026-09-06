// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v3-gameplay/schema.json"
));
const DIGEST: &str = "8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeV3GameplayForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV3GameplayForwardError {
    RequestBodyRequired,
    RequestBodyOversized,
    RequestBodyMalformed,
    ResponseOversized,
    ResponseMalformed,
}

impl RuntimeV3GameplayForwarder {
    pub(crate) const fn new(max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
        }
    }

    pub(crate) fn validate_request(
        self,
        route: RuntimeV3GameplayRoute,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<Value, RuntimeV3GameplayForwardError> {
        if body.len() > self.max_request_bytes {
            return Err(RuntimeV3GameplayForwardError::RequestBodyOversized);
        }
        if body.is_empty() {
            return Err(RuntimeV3GameplayForwardError::RequestBodyRequired);
        }
        let value =
            validate_envelope(body).ok_or(RuntimeV3GameplayForwardError::RequestBodyMalformed)?;
        if value["kind"].as_str() != Some(route.request_kind()) || !headers_match(&value, headers) {
            return Err(RuntimeV3GameplayForwardError::RequestBodyMalformed);
        }
        Ok(value)
    }

    pub(crate) fn validate_response(
        self,
        route: RuntimeV3GameplayRoute,
        request: &Value,
        body: &[u8],
    ) -> Result<(), RuntimeV3GameplayForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV3GameplayForwardError::ResponseOversized);
        }
        let value =
            validate_envelope(body).ok_or(RuntimeV3GameplayForwardError::ResponseMalformed)?;
        for field in [
            "correlation_id",
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
        ] {
            if value[field] != request[field] {
                return Err(RuntimeV3GameplayForwardError::ResponseMalformed);
            }
        }
        if value["kind"].as_str() != Some(route.response_kind()) {
            return Err(RuntimeV3GameplayForwardError::ResponseMalformed);
        }
        if matches!(
            route,
            RuntimeV3GameplayRoute::DispatchAction | RuntimeV3GameplayRoute::WaitForTransition
        ) && value["operation_id"] != request["operation_id"]
        {
            return Err(RuntimeV3GameplayForwardError::ResponseMalformed);
        }
        Ok(())
    }

    /// A catalog refusal is not a catalog. The host-owned HTTP error contract is
    /// deliberately narrower than the canonical success envelope and never grants admission.
    pub(crate) fn is_legal_actions_recovery(
        self,
        route: RuntimeV3GameplayRoute,
        request: &Value,
        status: u16,
        body: &[u8],
    ) -> bool {
        if route != RuntimeV3GameplayRoute::LegalActions || body.len() > 1024 {
            return false;
        }
        let Ok(value) = super::strict_json::parse(body) else {
            return false;
        };
        let Some(object) = value.as_object() else {
            return false;
        };
        object.len() == 3
            && value["correlation_id"] == request["correlation_id"]
            && value["recovery"].as_str() == Some("reobserve")
            && matches!(
                (status, value["error_code"].as_str()),
                (409, Some("stale_generation"))
                    | (
                        503,
                        Some("host_not_configured" | "host_observation_unavailable")
                    )
            )
    }
}

fn headers_match(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    for (field, header) in [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ] {
        if value[field].as_str() != headers.get(header).map(String::as_str) {
            return false;
        }
    }
    value["lease_epoch"].as_u64()
        == headers
            .get("x-sts2-lease-epoch")
            .and_then(|epoch| epoch.parse::<u64>().ok())
}

fn validate_envelope(body: &[u8]) -> Option<Value> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()?;
    let value: Value = super::strict_json::parse(body).ok()?;
    (value["schema_digest"].as_str() == Some(DIGEST)
        && validator.is_valid(&value)
        && super::runtime_v3_relations::valid(&value))
    .then_some(value)
}

#[cfg(test)]
#[path = "runtime_v3_forwarder_tests.rs"]
mod tests;
