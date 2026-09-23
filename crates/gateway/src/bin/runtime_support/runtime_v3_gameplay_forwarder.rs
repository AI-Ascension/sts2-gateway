// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v3-gameplay/schema.json"
));
const DIGEST: &str = "daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b";

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
        if route == RuntimeV3GameplayRoute::LegalActions
            && !legal_actions_response_matches_request(request, &value)
        {
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
            && admits_recovery_code(status, value["error_code"].as_str())
    }
}

/// The recovery codes the legal-action route admits, paired with the status each arrives with.
///
/// A refused launch contract reaches this route as `503` carrying a code the mod composes from its
/// own refusal prefix plus an optional bounded reason token (`sts2-game-mod#185`, `#187`). The set
/// here is the producer's vocabulary, not a second one: see [`is_launch_contract_refusal`].
fn admits_recovery_code(status: u16, code: Option<&str>) -> bool {
    match (status, code) {
        (409, Some("stale_generation")) => true,
        (503, Some("host_not_configured" | "host_observation_unavailable")) => true,
        (503, Some(code)) => is_launch_contract_refusal(code),
        _ => false,
    }
}

/// True for the recovery code a refused launch contract carries.
///
/// The mod answers the bare prefix (`launch_contract_refused`) when a reason cannot be named on the
/// wire, and otherwise the prefix, `_`, and one reason token. The token rule is mirrored from the
/// producer so a code it cannot emit is refused here too: widening this set must not admit a
/// neighbouring string that merely starts the same way.
fn is_launch_contract_refusal(code: &str) -> bool {
    const PREFIX: &str = "launch_contract_refused";
    /// Bounded exactly as the producer bounds it, so neither side can compose an unbounded code.
    const MAX_REASON_BYTES: usize = 64;
    let Some(reason) = code.strip_prefix(PREFIX) else {
        return false;
    };
    if reason.is_empty() {
        return true;
    }
    let Some(token) = reason.strip_prefix('_') else {
        return false;
    };
    !token.is_empty()
        && token.len() <= MAX_REASON_BYTES
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn legal_actions_response_matches_request(request: &Value, response: &Value) -> bool {
    let state_matches =
        request["state_id"].is_null() || request["state_id"] == response["state_id"];
    let generation_matches =
        request["generation"].is_null() || request["generation"] == response["generation"];
    state_matches && generation_matches
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
