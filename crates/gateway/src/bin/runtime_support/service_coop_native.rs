// SPDX-License-Identifier: MIT

use super::super::coop_native::CoopNativeRoute;
use super::super::coop_native_forwarder::{CoopNativeForwardError, CoopNativeForwarder};
use super::{HttpRequest, MAX_RESPONSE_BYTES, RuntimeService, json_error};

impl RuntimeService {
    pub(super) fn coop_native_request(
        &mut self,
        request: &HttpRequest,
        route: CoopNativeRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route != CoopNativeRoute::Observation && !request.content_type_is_json() {
            return (400, json_error("coop_native_content_type_required"));
        }
        let envelope =
            match self
                .coop_native
                .validate_request(route, &request.body, &request.headers)
            {
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
        let response = match self.forward_mod_with_limit(
            if route == CoopNativeRoute::Observation {
                "GET"
            } else {
                "POST"
            },
            route.downstream_path(),
            &request.body,
            correlation,
            MAX_RESPONSE_BYTES,
        ) {
            Ok(response) => response,
            Err(status) => return (status, json_error("coop_native_downstream_unavailable")),
        };
        if self
            .coop_native
            .validate_response(
                route,
                envelope.as_ref(),
                &request.headers,
                response.status,
                &response.body,
            )
            .is_ok()
        {
            return (response.status, response.body);
        }
        // The managed producer reports stale/admission failures as a bounded one-field HTTP
        // error rather than a protocol envelope. Preserve those explicit host decisions while
        // mapping every other malformed downstream response to 502.
        if CoopNativeForwarder::is_bounded_error(response.status, &response.body) {
            return (response.status, response.body);
        }
        (502, json_error("coop_native_response_invalid"))
    }
}

fn request_error_status(error: CoopNativeForwardError) -> u16 {
    match error {
        CoopNativeForwardError::RequestBodyOversized => 413,
        CoopNativeForwardError::RequestBodyRequired
        | CoopNativeForwardError::RequestBodyForbidden
        | CoopNativeForwardError::RequestBodyMalformed => 400,
        CoopNativeForwardError::ResponseOversized | CoopNativeForwardError::ResponseMalformed => {
            502
        }
    }
}

fn request_error_code(error: CoopNativeForwardError) -> &'static str {
    match error {
        CoopNativeForwardError::RequestBodyRequired => "coop_native_body_required",
        CoopNativeForwardError::RequestBodyForbidden => "coop_native_body_forbidden",
        CoopNativeForwardError::RequestBodyOversized => "coop_native_body_oversized",
        CoopNativeForwardError::RequestBodyMalformed => "coop_native_request_invalid",
        CoopNativeForwardError::ResponseOversized => "coop_native_response_oversized",
        CoopNativeForwardError::ResponseMalformed => "coop_native_response_invalid",
    }
}
