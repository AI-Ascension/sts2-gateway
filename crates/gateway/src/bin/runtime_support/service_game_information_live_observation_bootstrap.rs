// SPDX-License-Identifier: MIT

use super::super::super::game_information::GameInformationRoute;
use super::super::super::game_information_live_observation_bootstrap as bootstrap;
use super::super::super::http::ReadError;
use super::super::{HttpRequest, RuntimeService, json_error};
use super::errors::game_information_transport_error;

pub(super) fn forward(
    service: &mut RuntimeService,
    request: &HttpRequest,
    cancellation: &super::super::RequestCancellation,
) -> (u16, Vec<u8>) {
    if !service.config.game_information_live_bootstrap_enabled {
        return (
            503,
            json_error("game_information_live_observation_bootstrap_unavailable"),
        );
    }
    if !request.content_type_is_json() {
        return (
            400,
            json_error("game_information_live_observation_bootstrap_content_type_required"),
        );
    }
    let authority = service.game_information_authority();
    let Some(binding) = service.game_information_lookup_binding.as_ref() else {
        return (
            409,
            json_error("game_information_lookup_binding_discovery_required"),
        );
    };
    if binding.authority != authority
        || binding.content_manifest_id != authority.content_manifest_id
    {
        service.game_information_lookup_binding = None;
        return (
            409,
            json_error("game_information_lookup_binding_discovery_required"),
        );
    }
    let body = match bootstrap::validate_request(
        &request.body,
        &request.headers,
        &authority,
        Some(binding),
        &service.config.game_information_locale,
    ) {
        Ok(body) => body,
        Err(error) => return request_error(error),
    };
    let correlation = request
        .headers
        .get("x-sts2-correlation-id")
        .map(String::as_str);
    let response = match service.forward_mod_with_limit_detailed_timeout_cancelable(
        "POST",
        GameInformationRoute::LiveObservationBootstrap.downstream_path(),
        &request.body,
        correlation,
        bootstrap::MAX_BOOTSTRAP_RESPONSE_BYTES,
        service.game_information_exchange_timeout,
        cancellation,
    ) {
        Ok(response) => response,
        Err(ReadError::Unavailable) => {
            service.game_information_live_bootstrap_supported = Some(authority);
            service.game_information_live_bootstrap_transport_failed = true;
            return (
                503,
                json_error("game_information_live_observation_bootstrap_unavailable"),
            );
        }
        Err(error) => return game_information_transport_error(error),
    };
    if let Err(error) = service.check_lease(request) {
        return error;
    }
    if service.game_information_authority() != authority {
        return (
            409,
            json_error("game_information_live_observation_bootstrap_scope_rejected"),
        );
    }
    let Some(current_binding) = service.game_information_lookup_binding.as_ref() else {
        return (
            409,
            json_error("game_information_lookup_binding_discovery_required"),
        );
    };
    if current_binding.authority != authority
        || current_binding.content_manifest_id != authority.content_manifest_id
    {
        service.game_information_lookup_binding = None;
        return (
            409,
            json_error("game_information_lookup_binding_discovery_required"),
        );
    }
    let _validated = match bootstrap::validate_response(
        &response.body,
        &body,
        &request.headers,
        &authority,
        service.game_information_lookup_binding.as_ref(),
        &service.config.game_information_locale,
        response.status,
    ) {
        Ok(response) => response,
        Err(_) => {
            service.game_information_live_bootstrap_supported = Some(authority);
            service.game_information_live_bootstrap_transport_failed = true;
            return (
                502,
                json_error("game_information_live_observation_bootstrap_response_invalid"),
            );
        }
    };
    service.game_information_live_bootstrap_supported = Some(authority);
    service.game_information_live_bootstrap_transport_failed = false;
    (response.status, response.body)
}

fn request_error(error: bootstrap::Error) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        bootstrap::Error::Required => (
            400,
            "game_information_live_observation_bootstrap_body_required",
        ),
        bootstrap::Error::Oversized => (
            413,
            "game_information_live_observation_bootstrap_request_oversized",
        ),
        bootstrap::Error::Invalid => (
            400,
            "game_information_live_observation_bootstrap_request_invalid",
        ),
        bootstrap::Error::Scope => (
            409,
            "game_information_live_observation_bootstrap_scope_rejected",
        ),
    };
    (status, json_error(code))
}
