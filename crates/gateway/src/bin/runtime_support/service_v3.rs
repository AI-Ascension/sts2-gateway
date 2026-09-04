// SPDX-License-Identifier: MIT

use super::{
    HttpRequest, RuntimeService, RuntimeV3GameplayForwardError, RuntimeV3GameplayRoute, json_error,
};

impl RuntimeService {
    pub(super) fn runtime_v3_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeV3GameplayRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.content_type_is_json() {
            return (400, json_error("runtime_v3_content_type_required"));
        }
        let envelope =
            match self
                .runtime_v3
                .validate_request(route, &request.body, &request.headers)
            {
                Ok(envelope) => envelope,
                Err(error) => {
                    return (
                        runtime_v3_request_status(error),
                        json_error(runtime_v3_error_code(error)),
                    );
                }
            };
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod(
            if route.is_post() { "POST" } else { "GET" },
            route.downstream_path(),
            &request.body,
            correlation,
        ) {
            Ok(response) => {
                match self
                    .runtime_v3
                    .validate_response(route, &envelope, &response.body)
                {
                    Ok(()) => (response.status, response.body),
                    Err(error) => (502, json_error(runtime_v3_error_code(error))),
                }
            }
            Err(status) => (status, json_error("runtime_v3_downstream_unavailable")),
        }
    }
}
fn runtime_v3_request_status(error: RuntimeV3GameplayForwardError) -> u16 {
    match error {
        RuntimeV3GameplayForwardError::RequestBodyOversized => 413,
        RuntimeV3GameplayForwardError::RequestBodyRequired
        | RuntimeV3GameplayForwardError::RequestBodyMalformed => 400,
        RuntimeV3GameplayForwardError::ResponseOversized
        | RuntimeV3GameplayForwardError::ResponseMalformed => 502,
    }
}

fn runtime_v3_error_code(error: RuntimeV3GameplayForwardError) -> &'static str {
    match error {
        RuntimeV3GameplayForwardError::RequestBodyRequired => "runtime_v3_body_required",
        RuntimeV3GameplayForwardError::RequestBodyOversized => "runtime_v3_body_oversized",
        RuntimeV3GameplayForwardError::RequestBodyMalformed => "runtime_v3_request_invalid",
        RuntimeV3GameplayForwardError::ResponseOversized => "runtime_v3_response_oversized",
        RuntimeV3GameplayForwardError::ResponseMalformed => "runtime_v3_response_invalid",
    }
}
