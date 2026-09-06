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
        if let Err(error) = self
            .runtime_v4_expert
            .validate_request(route, &request.body)
        {
            return (400, json_error(runtime_v4_expert_error_code(error)));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod("GET", route.downstream_path(), &[], correlation) {
            Ok(response) if (200..300).contains(&response.status) => {
                match self
                    .runtime_v4_expert
                    .validate_response(route, &response.body)
                {
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

fn runtime_v4_expert_error_code(error: RuntimeV4ExpertForwardError) -> &'static str {
    match error {
        RuntimeV4ExpertForwardError::RequestBodyForbidden => "runtime_v4_expert_body_forbidden",
        RuntimeV4ExpertForwardError::ResponseOversized => "runtime_v4_expert_response_oversized",
        RuntimeV4ExpertForwardError::ResponseMalformed => "runtime_v4_expert_response_invalid",
    }
}
