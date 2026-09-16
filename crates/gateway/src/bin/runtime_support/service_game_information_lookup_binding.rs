// SPDX-License-Identifier: MIT

use super::super::super::game_information::GameInformationRoute;
use super::super::super::game_information_forwarder::MAX_RESPONSE_BYTES;
use super::super::super::game_information_lookup_binding::{self, BoundLookupBinding};
use super::super::json_error;
use super::super::{HttpRequest, RuntimeService};
use super::errors::{
    game_information_transport_error, lookup_binding_request_is_closed,
    lookup_binding_request_is_discovery,
};

pub(super) fn forward(
    service: &mut RuntimeService,
    request: &HttpRequest,
    cancellation: &super::super::RequestCancellation,
) -> (u16, Vec<u8>) {
    if !request.content_type_is_json() || !lookup_binding_request_is_closed(&request.body) {
        return (400, json_error("game_information_lookup_binding_invalid"));
    }
    let discovery = lookup_binding_request_is_discovery(&request.body);
    // A new discovery supersedes the previous scope witness even when its
    // producer exchange fails, so an old binding cannot authorize catalog use.
    if discovery {
        service.game_information_lookup_binding = None;
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
            if discovery && response.status == 200 {
                let Some((binding_id, content_manifest_id, authority_epoch)) =
                    game_information_lookup_binding::discovery_witness(
                        &response.body,
                        &request.body,
                    )
                else {
                    return (
                        502,
                        json_error("game_information_lookup_binding_response_invalid"),
                    );
                };
                let authority = service.game_information_authority();
                if content_manifest_id != authority.content_manifest_id {
                    return (
                        502,
                        json_error("game_information_lookup_binding_response_invalid"),
                    );
                }
                service.game_information_lookup_binding = Some(BoundLookupBinding {
                    authority,
                    binding_id,
                    content_manifest_id,
                    authority_epoch,
                });
            }
            (response.status, response.body)
        }
        Err(error) => game_information_transport_error(error),
    }
}
