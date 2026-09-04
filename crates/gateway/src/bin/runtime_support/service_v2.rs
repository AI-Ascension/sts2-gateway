// SPDX-License-Identifier: MIT

use super::{HttpRequest, MAX_BODY_BYTES, RuntimeService, json_bytes, json_error, safe_identity};
use serde_json::json;
use sts2_gateway::{
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2LedgerError, RuntimeV2Message,
    RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault,
};

impl RuntimeService {
    pub(super) fn runtime_v2_action(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if request.body.len() > MAX_BODY_BYTES {
            return (413, json_error("runtime_v2_body_oversized"));
        }
        let Ok(message) = serde_json::from_slice::<RuntimeV2Message>(&request.body) else {
            return (400, json_error("runtime_v2_request_invalid"));
        };
        if request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(message.correlation_id.as_str())
        {
            return (409, json_error("runtime_v2_correlation_mismatch"));
        }
        match self.runtime_v2.submit_action(message) {
            Ok(response) => (runtime_v2_status(&response), runtime_v2_bytes(&response)),
            Err(error) => runtime_v2_error(error),
        }
    }

    pub(super) fn runtime_v2_state(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let state_request = RuntimeV2Message::state_request(
            self.runtime_v2.binding().metadata().clone(),
            correlation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            self.runtime_v2.observation().generation,
        );
        if state_request.validate().is_err() {
            return (500, json_error("runtime_v2_state_request_invalid"));
        }
        (503, runtime_v2_state_unavailable(&state_request))
    }

    pub(super) fn runtime_v2_reconcile(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_identity(operation_id) {
            return (400, json_error("runtime_v2_operation_invalid"));
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let message = RuntimeV2Message::reconcile_request(
            self.runtime_v2.binding().metadata().clone(),
            correlation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            self.runtime_v2.observation().generation,
            operation_id,
        );
        match self.runtime_v2.reconcile(message) {
            Ok(response) => (runtime_v2_status(&response), runtime_v2_bytes(&response)),
            Err(error) => runtime_v2_error(error),
        }
    }

    pub(super) fn runtime_v2_action_path(&self) -> String {
        format!("/v2/instances/{}/action", self.config.instance_id)
    }

    pub(super) fn runtime_v2_state_path(&self) -> String {
        format!("/v2/instances/{}/state", self.config.instance_id)
    }

    pub(super) fn runtime_v2_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("/v2/instances/{}/operations/", self.config.instance_id);
        path.strip_prefix(&prefix)
            .filter(|operation_id| !operation_id.is_empty() && !operation_id.contains('/'))
    }
}
/// The attached v1 binary has no authorized Runtime-v2 host adapter yet.
/// Keeping this seam explicit makes the v2 routes safe: no guessed host path is contacted.
pub(super) struct UnconfiguredRuntimeV2Forwarder;

impl RuntimeV2ForwardingPort for UnconfiguredRuntimeV2Forwarder {
    fn forward_runtime_v2(
        &mut self,
        _request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        Err(RuntimeV2TransportFault::UnavailableBeforeWrite)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        _request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        Ok(None)
    }
}

fn runtime_v2_bytes(message: &RuntimeV2Message) -> Vec<u8> {
    match serde_json::to_vec(message) {
        Ok(bytes) => bytes,
        Err(_) => json_error("runtime_v2_serialization_failed"),
    }
}

fn runtime_v2_status(message: &RuntimeV2Message) -> u16 {
    match message.status {
        Some(RuntimeV2Status::Rejected)
            if message.error_code.as_deref() == Some("idempotency_conflict") =>
        {
            409
        }
        Some(RuntimeV2Status::Rejected | RuntimeV2Status::Cancelled | RuntimeV2Status::Unknown) => {
            200
        }
        Some(RuntimeV2Status::Accepted | RuntimeV2Status::Settled) => 200,
        None => 200,
    }
}

fn runtime_v2_state_unavailable(request: &RuntimeV2Message) -> Vec<u8> {
    json_bytes(&json!({
        "status": "unavailable",
        "error_code": "sts2.runtime/state_unavailable",
        "reason": "unconfigured_runtime_v2_forwarder",
        "request": request,
    }))
}

fn runtime_v2_error(error: RuntimeV2LedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RuntimeV2LedgerError::InvalidRequest(_) | RuntimeV2LedgerError::RequestDigest(_) => {
            (400, "runtime_v2_request_invalid")
        }
        RuntimeV2LedgerError::MissingOperationId => (400, "runtime_v2_operation_required"),
        RuntimeV2LedgerError::CapacityExceeded => (429, "runtime_v2_operation_capacity"),
        RuntimeV2LedgerError::OperationNotFound => (404, "runtime_v2_operation_not_found"),
        RuntimeV2LedgerError::OperationInProgress => (409, "runtime_v2_operation_in_progress"),
        RuntimeV2LedgerError::Fence(_) => (409, "runtime_v2_lease_fence_rejected"),
        RuntimeV2LedgerError::StaleGeneration { .. } => (409, "runtime_v2_stale_generation"),
        RuntimeV2LedgerError::ZeroCapacity => (500, "runtime_v2_operation_capacity_invalid"),
    };
    (status, json_error(code))
}
