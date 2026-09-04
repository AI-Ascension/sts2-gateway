// SPDX-License-Identifier: MIT

#[path = "runtime_v3_gameplay_forwarder.rs"]
mod forwarder;
#[path = "runtime_v3_gameplay_operations.rs"]
mod operations;
#[cfg(test)]
#[path = "runtime_v3_gameplay_tests.rs"]
mod tests;
#[path = "runtime_v3_gameplay_validation.rs"]
mod validation;
#[path = "runtime_v3_gameplay_wire.rs"]
mod wire;

use std::collections::BTreeMap;

use serde_json::json;

use super::http::{HttpRequest, MAX_BODY_BYTES};
use forwarder::{
    HttpRuntimeV3GameplayForwarder, RuntimeV3GameplayTransportError, response_body_is_bounded,
};
use validation::{
    ParsedAction, parse_action_request, validate_result_response, validate_state_response,
};

const RUNTIME_V3_GAMEPLAY_PROTOCOL_VERSION: &str = "runtime-v3-gameplay";
const RUNTIME_V3_GAMEPLAY_ARTIFACT: &str = "sts2-protocol/runtime-v3-gameplay";
const RUNTIME_V3_GAMEPLAY_SCHEMA_SOURCE: &str = "schemas/runtime-v3-gameplay.schema.json";
const RUNTIME_V3_GAMEPLAY_GENERATOR: &str = "hand-authored";
const RUNTIME_V3_GAMEPLAY_SCHEMA_DIGEST: &str =
    "c961bbde893f0422f80233d14ea9ae8b648ee9032136e5370aa5f6b949f6575e";
const RUNTIME_V3_GAMEPLAY_ACTION_ID: &str = "play_card";
const RUNTIME_V3_GAMEPLAY_EFFECT_KIND: &str = "play_card_settled";
const RUNTIME_V3_GAMEPLAY_MAX_GENERATION: u64 = 9_007_199_254_740_991;
const RUNTIME_V3_GAMEPLAY_MAX_TURN_INDEX: u16 = 1024;
const RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX: u16 = 64;
const RUNTIME_V3_GAMEPLAY_MAX_ENERGY: u16 = 999;
const RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT: u16 = 1024;
const RUNTIME_V3_GAMEPLAY_MAX_ENEMIES: usize = 16;

#[derive(Debug)]
pub(crate) struct RuntimeV3GameplayProxy {
    forwarder: HttpRuntimeV3GameplayForwarder,
    operations: BTreeMap<String, RuntimeV3GameplayOperation>,
    generation: u64,
    capacity: usize,
}

#[derive(Debug)]
struct RuntimeV3GameplayOperation {
    request_body: Vec<u8>,
    action: ParsedAction,
    response: Option<StoredResponse>,
}

#[derive(Clone, Debug)]
struct StoredResponse {
    status: u16,
    body: Vec<u8>,
}

impl RuntimeV3GameplayProxy {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        mod_address: &str,
        mod_token: &str,
        instance_id: &str,
        caller_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        capacity: usize,
    ) -> Self {
        Self {
            forwarder: HttpRuntimeV3GameplayForwarder::new(
                mod_address,
                mod_token,
                instance_id,
                caller_id,
                session_id,
                lease_id,
                lease_epoch,
            ),
            operations: BTreeMap::new(),
            generation: 0,
            capacity,
        }
    }

    pub(crate) fn state(
        &mut self,
        request: &HttpRequest,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
    ) -> (u16, Vec<u8>) {
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        match self.forwarder.forward_state(correlation_id) {
            Ok(response) if response_body_is_bounded(&response) => {
                match validate_state_response(
                    &response.body,
                    instance_id,
                    session_id,
                    lease_id,
                    lease_epoch,
                    correlation_id,
                ) {
                    Ok(generation) if generation >= self.generation => {
                        self.generation = self.generation.max(generation);
                        (200, response.body)
                    }
                    _ => (502, json_error("runtime_v3_downstream_response_invalid")),
                }
            }
            Ok(_) => (502, json_error("runtime_v3_downstream_response_oversized")),
            Err(error) => transport_error(error),
        }
    }

    pub(crate) fn action(
        &mut self,
        request: &HttpRequest,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
    ) -> (u16, Vec<u8>) {
        if request.body.len() > MAX_BODY_BYTES {
            return (413, json_error("runtime_v3_gameplay_body_oversized"));
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let parsed = match parse_action_request(
            &request.body,
            instance_id,
            session_id,
            lease_id,
            lease_epoch,
            correlation_id,
        ) {
            Ok(parsed) => parsed,
            Err(code) => return (400, json_error(code)),
        };
        if let Some(operation) = self.operations.get(&parsed.operation_id) {
            return if operation.request_body == parsed.canonical_body {
                operation.response.as_ref().map_or_else(
                    || (503, json_error("runtime_v3_operation_in_progress")),
                    |response| stored_response_bytes(response, correlation_id),
                )
            } else {
                (409, json_error("runtime_v3_idempotency_conflict"))
            };
        }
        if self.operations.len() >= self.capacity {
            return (429, json_overload("runtime_v3_gameplay_operation_capacity"));
        }
        if parsed.generation < self.generation {
            return (409, json_error("runtime_v3_stale_generation"));
        }
        let operation_id = parsed.operation_id.clone();
        self.operations.insert(
            operation_id.clone(),
            RuntimeV3GameplayOperation {
                request_body: parsed.canonical_body.clone(),
                action: parsed.clone(),
                response: None,
            },
        );
        let response = self.forwarder.forward_action(correlation_id, &request.body);
        self.accept_action_response(
            &operation_id,
            response,
            instance_id,
            session_id,
            lease_id,
            lease_epoch,
            correlation_id,
        )
    }
}

fn stored_response_bytes(response: &StoredResponse, correlation_id: &str) -> (u16, Vec<u8>) {
    match wire::rebind(&response.body, correlation_id) {
        Ok(body) => (response.status, body),
        Err(_) => (502, json_error("runtime_v3_downstream_response_invalid")),
    }
}

fn transport_error(error: RuntimeV3GameplayTransportError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RuntimeV3GameplayTransportError::Unavailable => (503, "runtime_v3_downstream_unavailable"),
        RuntimeV3GameplayTransportError::Timeout => (504, "runtime_v3_downstream_timeout"),
        RuntimeV3GameplayTransportError::Malformed => (502, "runtime_v3_downstream_malformed"),
        RuntimeV3GameplayTransportError::Uncertain => (503, "runtime_v3_transport_uncertain"),
    };
    (status, json_error(code))
}

fn json_error(code: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({ "error_code": code }))
        .unwrap_or_else(|_| b"{\"error_code\":\"serialization_failed\"}".to_vec())
}

fn json_overload(code: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "error_code": code,
        "retryable": true,
        "retry_after_ms": 1000
    }))
    .unwrap_or_else(|_| b"{\"error_code\":\"serialization_failed\"}".to_vec())
}
