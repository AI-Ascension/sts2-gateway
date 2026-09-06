// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::sync::OnceLock;

use super::runtime_v4_expert::RuntimeV4ExpertRoute;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v4-expert/schema.json"
));
const DIGEST: &str = "f0786b039396043a441323447ac44f7cc4c218071bc477722f3ec992ab295a8a";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeV4ExpertForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertForwardError {
    RequestBodyForbidden,
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
        route: RuntimeV4ExpertRoute,
        body: &[u8],
    ) -> Result<(), RuntimeV4ExpertForwardError> {
        let _ = (self.max_request_bytes, route);
        if body.is_empty() {
            Ok(())
        } else {
            Err(RuntimeV4ExpertForwardError::RequestBodyForbidden)
        }
    }

    pub(crate) fn validate_response(
        self,
        route: RuntimeV4ExpertRoute,
        body: &[u8],
    ) -> Result<(), RuntimeV4ExpertForwardError> {
        let _ = route;
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV4ExpertForwardError::ResponseOversized);
        }
        let value =
            validate_observation(body).ok_or(RuntimeV4ExpertForwardError::ResponseMalformed)?;
        if value["protocol_version"].as_str() != Some("runtime-v4-expert") {
            return Err(RuntimeV4ExpertForwardError::ResponseMalformed);
        }
        Ok(())
    }
}

fn validate_observation(body: &[u8]) -> Option<Value> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()?;
    let value: Value = super::strict_json::parse(body).ok()?;
    (value["schema_digest"].as_str() == Some(DIGEST) && validator.is_valid(&value)).then_some(value)
}

#[cfg(test)]
#[path = "runtime_v4_expert_forwarder_tests.rs"]
mod tests;
