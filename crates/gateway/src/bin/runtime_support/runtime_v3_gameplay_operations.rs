// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::{
    HttpRequest, RuntimeV3GameplayProxy, RuntimeV3GameplayTransportError, StoredResponse,
    json_error, response_body_is_bounded, stored_response_bytes, transport_error,
    validate_result_response, wire,
};

impl RuntimeV3GameplayProxy {
    pub(crate) fn operation(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
    ) -> (u16, Vec<u8>) {
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let Some(operation) = self.operations.get(operation_id) else {
            return (404, json_error("runtime_v3_operation_not_found"));
        };
        if let Some(stored_response) = operation
            .response
            .as_ref()
            .filter(|response| wire::terminal(&response.body))
        {
            return stored_response_bytes(stored_response, correlation_id);
        }
        let action = operation.action.clone();
        let response = self
            .forwarder
            .forward_operation(correlation_id, operation_id);
        match response {
            Ok(response) if response_body_is_bounded(&response) => {
                if validate_result_response(
                    &response.body,
                    instance_id,
                    session_id,
                    lease_id,
                    lease_epoch,
                    correlation_id,
                    operation_id,
                    &action,
                )
                .is_err()
                {
                    return (502, json_error("runtime_v3_downstream_response_invalid"));
                }
                let stored = StoredResponse {
                    status: response.status,
                    body: response.body,
                };
                self.generation = self.generation.max(generation(&stored.body));
                if let Some(operation) = self.operations.get_mut(operation_id) {
                    operation.response = Some(stored.clone());
                }
                stored_response_bytes(&stored, correlation_id)
            }
            Ok(_) => (502, json_error("runtime_v3_downstream_response_oversized")),
            Err(error) => transport_error(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn accept_action_response(
        &mut self,
        operation_id: &str,
        response: Result<super::super::http::HttpResponse, RuntimeV3GameplayTransportError>,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        correlation_id: &str,
    ) -> (u16, Vec<u8>) {
        let action = self
            .operations
            .get(operation_id)
            .map(|operation| operation.action.clone());
        let Some(action) = action else {
            return (500, json_error("runtime_v3_operation_lost"));
        };
        let response = match response {
            Ok(response) => response,
            Err(RuntimeV3GameplayTransportError::Unavailable) => {
                self.operations.remove(operation_id);
                return transport_error(RuntimeV3GameplayTransportError::Unavailable);
            }
            Err(error) => return transport_error(error),
        };
        if !response_body_is_bounded(&response) {
            return (502, json_error("runtime_v3_downstream_response_oversized"));
        }
        if validate_result_response(
            &response.body,
            instance_id,
            session_id,
            lease_id,
            lease_epoch,
            correlation_id,
            operation_id,
            &action,
        )
        .is_err()
        {
            return (502, json_error("runtime_v3_downstream_response_invalid"));
        }
        let stored = StoredResponse {
            status: response.status,
            body: response.body,
        };
        self.generation = self.generation.max(generation(&stored.body));
        if let Some(operation) = self.operations.get_mut(operation_id) {
            operation.response = Some(stored.clone());
        }
        stored_response_bytes(&stored, correlation_id)
    }
}

fn generation(body: &[u8]) -> u64 {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("generation").and_then(Value::as_u64))
        .unwrap_or(0)
}
