// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_rest_action_forwarder::RuntimeV4ExpertRestActionForwardError;
use super::{HttpRequest, RuntimeService, RuntimeV4ExpertRestActionRoute, json_error};

impl RuntimeService {
    pub(super) fn runtime_v4_expert_rest_action_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeV4ExpertRestActionRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route.is_dispatch() && !request.content_type_is_json() {
            return (
                400,
                json_error("runtime_v4_expert_rest_action_content_type_required"),
            );
        }
        let envelope = match self.runtime_v4_expert_rest_action.validate_request(
            &route,
            &request.body,
            &request.headers,
        ) {
            Ok(envelope) => envelope,
            Err(error) => {
                return (
                    request_error_status(error),
                    json_error(request_error_code(error)),
                );
            }
        };
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let method = if route.is_dispatch() { "POST" } else { "GET" };
        match self.forward_mod(method, &route.downstream_path(), &request.body, correlation) {
            Ok(response) => {
                let candidate_status = match self
                    .runtime_v4_expert_rest_action
                    .candidate_http_status(response.status, &response.body)
                {
                    Ok(status) => status,
                    Err(error) => return (502, json_error(response_error_code(error))),
                };
                match self.runtime_v4_expert_rest_action.validate_response(
                    &route,
                    &envelope,
                    &request.headers,
                    candidate_status,
                    &response.body,
                ) {
                    Ok(()) => (candidate_status, response.body),
                    Err(error) => (502, json_error(response_error_code(error))),
                }
            }
            Err(status) => (
                status,
                json_error("runtime_v4_expert_rest_action_downstream_unavailable"),
            ),
        }
    }
}

fn request_error_status(error: RuntimeV4ExpertRestActionForwardError) -> u16 {
    match error {
        RuntimeV4ExpertRestActionForwardError::RequestBodyOversized => 413,
        RuntimeV4ExpertRestActionForwardError::RequestBodyRequired
        | RuntimeV4ExpertRestActionForwardError::RequestBodyForbidden
        | RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed => 400,
        RuntimeV4ExpertRestActionForwardError::ResponseOversized
        | RuntimeV4ExpertRestActionForwardError::ResponseMalformed => 502,
    }
}

fn request_error_code(error: RuntimeV4ExpertRestActionForwardError) -> &'static str {
    match error {
        RuntimeV4ExpertRestActionForwardError::RequestBodyRequired => {
            "runtime_v4_expert_rest_action_body_required"
        }
        RuntimeV4ExpertRestActionForwardError::RequestBodyForbidden => {
            "runtime_v4_expert_rest_action_body_forbidden"
        }
        RuntimeV4ExpertRestActionForwardError::RequestBodyOversized => {
            "runtime_v4_expert_rest_action_body_oversized"
        }
        RuntimeV4ExpertRestActionForwardError::RequestBodyMalformed => {
            "runtime_v4_expert_rest_action_request_invalid"
        }
        RuntimeV4ExpertRestActionForwardError::ResponseOversized => {
            "runtime_v4_expert_rest_action_response_oversized"
        }
        RuntimeV4ExpertRestActionForwardError::ResponseMalformed => {
            "runtime_v4_expert_rest_action_response_invalid"
        }
    }
}

fn response_error_code(error: RuntimeV4ExpertRestActionForwardError) -> &'static str {
    match error {
        RuntimeV4ExpertRestActionForwardError::ResponseOversized => {
            "runtime_v4_expert_rest_action_response_oversized"
        }
        RuntimeV4ExpertRestActionForwardError::ResponseMalformed => {
            "runtime_v4_expert_rest_action_response_invalid"
        }
        _ => "runtime_v4_expert_rest_action_response_invalid",
    }
}
