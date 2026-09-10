// SPDX-License-Identifier: MIT

use super::{
    HttpRequest, MAX_MAP_RESPONSE_BYTES, RuntimeMapForwardError, RuntimeMapRoute, RuntimeService,
    json_error,
};

impl RuntimeService {
    pub(super) fn runtime_map_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeMapRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.body.is_empty() {
            return (400, json_error("runtime_map_body_forbidden"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit(
            "GET",
            route.downstream_path(),
            &[],
            correlation,
            MAX_MAP_RESPONSE_BYTES,
        ) {
            Ok(response) => response,
            Err(status) => return (status, json_error("runtime_map_downstream_unavailable")),
        };
        match self
            .runtime_map
            .validate_response(route, &request.headers, &response.body)
        {
            Ok(_) => (response.status, response.body),
            Err(error) => (502, json_error(map_response_error_code(error))),
        }
    }
}

fn map_response_error_code(error: RuntimeMapForwardError) -> &'static str {
    match error {
        RuntimeMapForwardError::ResponseOversized => "runtime_map_response_oversized",
        RuntimeMapForwardError::ResponseMalformed => "runtime_map_response_invalid",
    }
}
