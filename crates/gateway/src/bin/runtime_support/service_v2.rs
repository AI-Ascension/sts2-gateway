// SPDX-License-Identifier: MIT

use super::*;

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
        if message
            .operation_id
            .as_deref()
            .is_some_and(|id| !safe_operation_id(id))
        {
            return (400, json_error("runtime_v2_operation_invalid"));
        }
        if request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(message.correlation_id.as_str())
        {
            return (409, json_error("runtime_v2_correlation_mismatch"));
        }
        let result = match self.journal_path.as_deref() {
            Some(path) => self
                .runtime_v2
                .submit_action_with_checkpoint(message, |state| {
                    journal::store(path, state).map_err(|_| ())
                }),
            None => self.runtime_v2.submit_action(message),
        };
        match result {
            Ok(response) => {
                if response.status == Some(RuntimeV2Status::Unknown) {
                    self.metrics.runtime_v2_unknown();
                }
                (runtime_v2_status(&response), runtime_v2_bytes(&response))
            }
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
        let state_request = if request.body.is_empty() {
            RuntimeV2Message::state_request(
                self.runtime_v2.binding().metadata().clone(),
                correlation_id,
                &self.config.instance_id,
                &self.config.session_id,
                &self.config.lease_id,
                self.config.lease_epoch,
                self.runtime_v2.observation().generation,
            )
        } else {
            let Ok(message) = serde_json::from_slice::<RuntimeV2Message>(&request.body) else {
                return (400, json_error("runtime_v2_state_request_invalid"));
            };
            if message.kind != sts2_gateway::RuntimeV2MessageKind::StateRequest
                || message.validate().is_err()
                || message.correlation_id != correlation_id.as_str()
                || message.instance_id != self.config.instance_id
                || message.session_id != self.config.session_id
                || message.lease_id != self.config.lease_id
                || message.lease_epoch != self.config.lease_epoch
            {
                return (409, json_error("runtime_v2_state_identity_rejected"));
            }
            message
        };
        if state_request.validate().is_err() {
            return (500, json_error("runtime_v2_state_request_invalid"));
        }
        match self
            .runtime_v2
            .forwarding_mut()
            .forward_state(state_request.clone())
        {
            Ok(response) => match self
                .runtime_v2
                .accept_state_response(&state_request, response)
            {
                Ok(response) => {
                    if let Some(path) = self.journal_path.as_deref()
                        && journal::store(path, &self.runtime_v2.persisted_state()).is_err()
                    {
                        return (
                            503,
                            runtime_v2_state_unavailable(
                                &state_request,
                                "runtime_v2_persistence_failed",
                            ),
                        );
                    }
                    (200, runtime_v2_bytes(&response))
                }
                Err(_) => (
                    502,
                    runtime_v2_state_unavailable(
                        &state_request,
                        "downstream_state_response_invalid",
                    ),
                ),
            },
            Err(error) => (
                runtime_v2_state_status(error),
                runtime_v2_state_unavailable(&state_request, runtime_v2_state_reason(error)),
            ),
        }
    }

    pub(super) fn runtime_v2_reconcile(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_operation_id(operation_id) {
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
            Ok(response) => {
                if response.status == Some(RuntimeV2Status::Unknown) {
                    self.metrics.runtime_v2_unknown();
                }
                if let Some(path) = self.journal_path.as_deref()
                    && journal::store(path, &self.runtime_v2.persisted_state()).is_err()
                {
                    return (503, json_error("runtime_v2_persistence_failed"));
                }
                (runtime_v2_status(&response), runtime_v2_bytes(&response))
            }
            Err(error) => runtime_v2_error(error),
        }
    }
}
pub(super) fn runtime_v2_bytes(message: &RuntimeV2Message) -> Vec<u8> {
    match serde_json::to_vec(message) {
        Ok(bytes) => bytes,
        Err(_) => json_error("runtime_v2_serialization_failed"),
    }
}

pub(super) fn runtime_v2_status(message: &RuntimeV2Message) -> u16 {
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

pub(super) fn runtime_v2_state_unavailable(request: &RuntimeV2Message, reason: &str) -> Vec<u8> {
    json_bytes(&json!({
        "status": "unavailable",
        "error_code": "sts2.runtime/state_unavailable",
        "reason": reason,
        "request": request,
    }))
}

pub(super) fn runtime_v2_state_status(error: RuntimeV2TransportFault) -> u16 {
    match error {
        RuntimeV2TransportFault::TimeoutBeforeWrite
        | RuntimeV2TransportFault::TimeoutAfterWrite => 504,
        RuntimeV2TransportFault::MalformedResponse => 502,
        RuntimeV2TransportFault::UnavailableBeforeWrite
        | RuntimeV2TransportFault::RejectedBeforeWrite
        | RuntimeV2TransportFault::DisconnectedBeforeWrite
        | RuntimeV2TransportFault::DisconnectedAfterWrite
        | RuntimeV2TransportFault::ReceiptUnavailable => 503,
    }
}

pub(super) fn runtime_v2_state_reason(error: RuntimeV2TransportFault) -> &'static str {
    match error {
        RuntimeV2TransportFault::UnavailableBeforeWrite => "downstream_unavailable_before_write",
        RuntimeV2TransportFault::RejectedBeforeWrite => "downstream_rejected_before_write",
        RuntimeV2TransportFault::TimeoutBeforeWrite => "downstream_timeout_before_write",
        RuntimeV2TransportFault::DisconnectedBeforeWrite => "downstream_disconnected_before_write",
        RuntimeV2TransportFault::TimeoutAfterWrite => "downstream_timeout_after_write",
        RuntimeV2TransportFault::DisconnectedAfterWrite => "downstream_disconnected_after_write",
        RuntimeV2TransportFault::MalformedResponse => "downstream_malformed_response",
        RuntimeV2TransportFault::ReceiptUnavailable => "downstream_receipt_unavailable",
    }
}

pub(super) fn runtime_v2_error(error: RuntimeV2LedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RuntimeV2LedgerError::InvalidRequest(_) | RuntimeV2LedgerError::RequestDigest(_) => {
            (400, "runtime_v2_request_invalid")
        }
        RuntimeV2LedgerError::MissingOperationId => (400, "runtime_v2_operation_required"),
        RuntimeV2LedgerError::CapacityExceeded => {
            return (429, json_overload("runtime_v2_operation_capacity"));
        }
        RuntimeV2LedgerError::OperationNotFound => (404, "runtime_v2_operation_not_found"),
        RuntimeV2LedgerError::OperationInProgress => (409, "runtime_v2_operation_in_progress"),
        RuntimeV2LedgerError::Fence(_) => (409, "runtime_v2_lease_fence_rejected"),
        RuntimeV2LedgerError::StaleGeneration { .. } => (409, "runtime_v2_stale_generation"),
        RuntimeV2LedgerError::ZeroCapacity => (500, "runtime_v2_operation_capacity_invalid"),
        RuntimeV2LedgerError::PersistedStateMismatch
        | RuntimeV2LedgerError::PersistedStateInvalid => (500, "runtime_v2_journal_invalid"),
        RuntimeV2LedgerError::PersistenceFailed => (503, "runtime_v2_persistence_failed"),
    };
    (status, json_error(code))
}
