// SPDX-License-Identifier: MIT

use super::super::runtime_v4_expert_forwarder::RuntimeV4ExpertForwardError;
use super::{HttpRequest, RuntimeService, RuntimeV4ExpertRoute, json_error};

impl RuntimeService {
    pub(super) fn runtime_v4_expert_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeV4ExpertRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route.is_dispatch() && !request.content_type_is_json() {
            return (400, json_error("runtime_v4_expert_content_type_required"));
        }
        let envelope =
            match self
                .runtime_v4_expert
                .validate_request(&route, &request.body, &request.headers)
            {
                Ok(envelope) => envelope,
                Err(error) => {
                    return (
                        runtime_v4_expert_request_status(error),
                        json_error(runtime_v4_expert_error_code(error)),
                    );
                }
            };
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let method = if route.is_dispatch() { "POST" } else { "GET" };
        let downstream_path = route.downstream_path();
        // The native mod route requires an empty GET body and copies the operation suffix from
        // `downstream_path` into the callback body before RuntimeV4ExpertSupport.Handle runs.
        match self.forward_mod(method, &downstream_path, &request.body, correlation) {
            Ok(response) if route.is_state() && (200..300).contains(&response.status) => match self
                .runtime_v4_expert
                .validate_response(&route, &envelope, &request.headers, &response.body)
            {
                Ok(()) => (response.status, response.body),
                Err(error) => (502, json_error(runtime_v4_expert_error_code(error))),
            },
            Ok(response) if !route.is_state() => {
                match self.runtime_v4_expert.validate_response(
                    &route,
                    &envelope,
                    &request.headers,
                    &response.body,
                ) {
                    Ok(()) => (response.status, response.body),
                    Err(error) => (502, json_error(runtime_v4_expert_error_code(error))),
                }
            }
            Ok(response) => (response.status, response.body),
            Err(status) => (
                status,
                json_error("runtime_v4_expert_downstream_unavailable"),
            ),
        }
    }
}

fn runtime_v4_expert_request_status(error: RuntimeV4ExpertForwardError) -> u16 {
    match error {
        RuntimeV4ExpertForwardError::RequestBodyOversized => 413,
        RuntimeV4ExpertForwardError::RequestBodyRequired
        | RuntimeV4ExpertForwardError::RequestBodyMalformed
        | RuntimeV4ExpertForwardError::RequestBodyForbidden => 400,
        RuntimeV4ExpertForwardError::ResponseOversized
        | RuntimeV4ExpertForwardError::ResponseMalformed => 502,
    }
}

fn runtime_v4_expert_error_code(error: RuntimeV4ExpertForwardError) -> &'static str {
    match error {
        RuntimeV4ExpertForwardError::RequestBodyForbidden => "runtime_v4_expert_body_forbidden",
        RuntimeV4ExpertForwardError::RequestBodyRequired => "runtime_v4_expert_body_required",
        RuntimeV4ExpertForwardError::RequestBodyOversized => "runtime_v4_expert_body_oversized",
        RuntimeV4ExpertForwardError::RequestBodyMalformed => "runtime_v4_expert_request_invalid",
        RuntimeV4ExpertForwardError::ResponseOversized => "runtime_v4_expert_response_oversized",
        RuntimeV4ExpertForwardError::ResponseMalformed => "runtime_v4_expert_response_invalid",
    }
}
