// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;

use super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;

#[path = "runtime_v4_expert_rest_action_forwarder_operations.rs"]
mod operations;
#[cfg(test)]
pub(super) use operations::MAX_OPERATION_BINDINGS;
use operations::OperationBinding;

#[path = "runtime_v4_expert_rest_action_observation.rs"]
mod observation;
use observation::observation_valid;
#[path = "runtime_v4_expert_rest_action_forwarder_wire.rs"]
mod wire;
use wire::{action_admitted, object_json, request_valid, response_identity_valid, schema_valid};

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v4-expert-rest-action/schema.json"
));
const PROTOCOL_VERSION: &str = "runtime-v4-expert-rest-action-v1";
const SCHEMA_DIGEST: &str = "bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd";
const PROFILE: &str = "expert-rest-action";
const NATIVE_REQUEST_BODY_BYTES: usize = 16 * 1024;
const NATIVE_IDENTITY_BYTES: usize = 128;

use super::runtime_v4_expert_rest_action_semantics::SelectorAdmission;

const MAX_SELECTOR_ADMISSIONS: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeV4ExpertRestActionForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
    selector_admissions: BTreeMap<String, SelectorAdmission>,
    operation_bindings: BTreeMap<String, OperationBinding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertRestActionForwardError {
    RequestBodyRequired,
    RequestBodyForbidden,
    RequestBodyOversized,
    RequestBodyMalformed,
    OperationCapacity,
    ResponseOversized,
    ResponseMalformed,
}

impl RuntimeV4ExpertRestActionForwarder {
    pub(crate) fn new(max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
            selector_admissions: BTreeMap::new(),
            operation_bindings: BTreeMap::new(),
        }
    }

    pub(crate) fn validate_request(
        &mut self,
        route: &RuntimeV4ExpertRestActionRoute,
        body: &[u8],
        headers: &BTreeMap<String, String>,
    ) -> Result<Value, RuntimeV4ExpertRestActionForwardError> {
        if body.len() > self.max_request_bytes || body.len() > NATIVE_REQUEST_BODY_BYTES {
            return Err(RuntimeV4ExpertRestActionForwardError::RequestBodyOversized);
        }
        if !route.is_dispatch() {
            return if body.is_empty() {
                Ok(Value::Null)
            } else {
                Err(RuntimeV4ExpertRestActionForwardError::RequestBodyForbidden)
            };
        }
        if body.is_empty() {
            return Err(RuntimeV4ExpertRestActionForwardError::RequestBodyRequired);
        }
        let value =
            object_json(body).ok_or(RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed)?;
        if !request_valid(&value, headers) {
            return Err(RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed);
        }
        if let Some(operation_id) = value["operation_id"].as_str() {
            let replay = self
                .operation_bindings
                .get(operation_id)
                .is_some_and(|binding| {
                    binding.action == value["action"]
                        && binding.generation == value["generation"].as_u64().unwrap_or(u64::MAX)
                        && binding.state_id == value["state_id"]
                        && binding.instance_id == value["instance_id"]
                        && binding.session_id == value["session_id"]
                        && binding.lease_id == value["lease_id"]
                        && binding.lease_epoch == value["lease_epoch"].as_u64().unwrap_or(u64::MAX)
                });
            if !replay && !action_admitted(&value, &self.selector_admissions) {
                return Err(RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed);
            }
            if self
                .operation_bindings
                .get(operation_id)
                .is_some_and(|binding| {
                    binding.action != value["action"]
                        || binding.generation != value["generation"].as_u64().unwrap_or(u64::MAX)
                        || binding.state_id != value["state_id"]
                        || binding.instance_id != value["instance_id"]
                        || binding.session_id != value["session_id"]
                        || binding.lease_id != value["lease_id"]
                        || binding.lease_epoch != value["lease_epoch"].as_u64().unwrap_or(u64::MAX)
                })
            {
                return Err(RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed);
            }
            if !self.operation_bindings.contains_key(operation_id)
                && !self.retain_operation_binding(operation_id, &value)
            {
                return Err(RuntimeV4ExpertRestActionForwardError::OperationCapacity);
            }
        }
        Ok(value)
    }

    pub(crate) fn validate_response(
        &mut self,
        route: &RuntimeV4ExpertRestActionRoute,
        request: &Value,
        headers: &BTreeMap<String, String>,
        status_code: u16,
        body: &[u8],
    ) -> Result<(), RuntimeV4ExpertRestActionForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV4ExpertRestActionForwardError::ResponseOversized);
        }
        let value =
            object_json(body).ok_or(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)?;
        let expected_action = request["action"]
            .as_object()
            .map(|_| &request["action"])
            .or_else(|| {
                route.operation_id().and_then(|operation_id| {
                    self.operation_bindings
                        .get(operation_id)
                        .map(|binding| &binding.action)
                })
            });
        if !response_identity_valid(&value, route, request, headers, expected_action)
            || !schema_valid(&value)
            || !super::runtime_v4_expert_rest_action_semantics::response_valid(
                &value,
                request,
                route.operation_id(),
                status_code,
                &self.selector_admissions,
                observation_valid,
            )
            || !self.reconciliation_binding_valid(route, &value)
        {
            return Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed);
        }
        self.record_selector_admission(&value);
        self.record_operation_status(route, request, &value);
        Ok(())
    }

    pub(crate) fn candidate_http_status(
        &self,
        downstream_status: u16,
        body: &[u8],
    ) -> Result<u16, RuntimeV4ExpertRestActionForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV4ExpertRestActionForwardError::ResponseOversized);
        }
        let value =
            object_json(body).ok_or(RuntimeV4ExpertRestActionForwardError::ResponseMalformed)?;
        let envelope_status = value["status"].as_str();
        let candidate_status = match envelope_status {
            Some("accepted") => 202,
            Some("settled") => 200,
            Some("rejected") => 409,
            Some("unknown") => match downstream_status {
                200 => 404,
                404 | 408 | 502 | 504 => downstream_status,
                // A native service can report an unavailable reconciliation
                // result as 503. Preserve the protocol's unknown envelope
                // while returning the gateway's canonical bad-gateway code.
                503 => 502,
                _ => return Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed),
            },
            Some("cancelled") => 499,
            _ => return Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed),
        };
        if downstream_status != 200
            && downstream_status != candidate_status
            && !(envelope_status == Some("unknown") && downstream_status == 503)
        {
            return Err(RuntimeV4ExpertRestActionForwardError::ResponseMalformed);
        }
        Ok(candidate_status)
    }

    fn record_selector_admission(&mut self, value: &Value) {
        let Some(transition) = value["transition"].as_object() else {
            return;
        };
        if !matches!(
            transition.get("kind").and_then(Value::as_str),
            Some("rest_option_selection_requested" | "rest_option_selection_progressed")
        ) {
            return;
        }
        let Some((selection_id, admission)) =
            super::runtime_v4_expert_rest_action_semantics::admission_from_transition(
                &Value::Object(transition.clone()),
            )
        else {
            return;
        };
        if self.selector_admissions.len() >= MAX_SELECTOR_ADMISSIONS
            && !self.selector_admissions.contains_key(&selection_id)
        {
            return;
        }
        self.selector_admissions.insert(selection_id, admission);
    }
}

#[cfg(test)]
#[path = "runtime_v4_expert_rest_action_forwarder_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_v4_expert_rest_action_mutation_tests.rs"]
mod mutation_tests;
#[cfg(test)]
#[path = "runtime_v4_expert_rest_action_producer_tests.rs"]
mod producer_tests;
#[cfg(test)]
#[path = "runtime_v4_expert_rest_action_forwarder_retention_tests.rs"]
mod retention_tests;
