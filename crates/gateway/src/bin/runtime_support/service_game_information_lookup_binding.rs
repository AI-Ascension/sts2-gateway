// SPDX-License-Identifier: MIT

use super::super::super::game_information::GameInformationRoute;
use super::super::super::game_information_forwarder::MAX_RESPONSE_BYTES;
use super::super::super::game_information_lookup_binding;
use super::super::json_error;
use super::super::{HttpRequest, RuntimeService};
use super::errors::{game_information_transport_error, lookup_binding_request_is_closed};

pub(super) fn forward(
    service: &RuntimeService,
    request: &HttpRequest,
    cancellation: &super::super::RequestCancellation,
) -> (u16, Vec<u8>) {
    if !request.content_type_is_json() || !lookup_binding_request_is_closed(&request.body) {
        return (400, json_error("game_information_lookup_binding_invalid"));
    }
    let correlation = request
        .headers
        .get("x-sts2-correlation-id")
        .map(String::as_str);
    let instance_id = request
        .headers
        .get("x-sts2-instance-id")
        .map(String::as_str)
        .unwrap_or_default();
    match service.forward_mod_with_limit_detailed_timeout_cancelable(
        "POST",
        GameInformationRoute::LookupBinding.downstream_path(),
        &request.body,
        correlation,
        MAX_RESPONSE_BYTES,
        service.game_information_exchange_timeout,
        cancellation,
    ) {
        Ok(response) => {
            if !game_information_lookup_binding::response_is_valid(
                &response.body,
                &request.body,
                correlation.unwrap_or(""),
                instance_id,
                &service.config.game_information_locale,
                response.status,
            ) {
                return (
                    502,
                    json_error("game_information_lookup_binding_response_invalid"),
                );
            }
            (response.status, response.body)
        }
        Err(error) => game_information_transport_error(error),
    }
}
