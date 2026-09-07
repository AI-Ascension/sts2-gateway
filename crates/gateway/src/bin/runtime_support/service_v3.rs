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
        if self.recovery.is_some() && route == RuntimeV3GameplayRoute::DispatchAction {
            return self.recovery_v3_dispatch(request, route, envelope);
        }
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
            Ok(response)
                if self.runtime_v3.is_legal_actions_recovery(
                    route,
                    &envelope,
                    response.status,
                    &response.body,
                ) =>
            {
                (response.status, response.body)
            }
            Ok(response) => {
                match self
                    .runtime_v3
                    .validate_response(route, &envelope, &response.body)
                {
                    Ok(()) => {
                        if let Some(lease) = self.recovery_lease.clone() {
                            // Only a schema-, relation-, and authority-context-validated
                            // response can change the state-scoped recovery catalog. State and
                            // reobserve responses establish freshness but never become the
                            // executable catalog themselves.
                            let catalog_update = match route {
                                RuntimeV3GameplayRoute::LegalActions => self
                                    .capture_recovery_catalog(
                                        &lease,
                                        response.status,
                                        &response.body,
                                    ),
                                RuntimeV3GameplayRoute::State
                                | RuntimeV3GameplayRoute::Reobserve => self
                                    .observe_recovery_catalog(
                                        &lease,
                                        response.status,
                                        &response.body,
                                    ),
                                _ => true,
                            };
                            if !catalog_update {
                                return (502, json_error("runtime_v3_catalog_observation_invalid"));
                            }
                        }
                        (response.status, response.body)
                    }
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
