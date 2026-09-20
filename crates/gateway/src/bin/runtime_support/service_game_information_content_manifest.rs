// SPDX-License-Identifier: MIT

use super::super::super::game_information::GameInformationRoute;
use super::super::super::game_information_content_manifest as manifest;
use super::super::super::http::ReadError;
use super::super::{HttpRequest, RuntimeService, json_error};

/// Forwards the fixed whole-manifest read and relays only a pinned, bounded, correlated envelope.
pub(super) fn forward(
    service: &mut RuntimeService,
    request: &HttpRequest,
    cancellation: &super::super::RequestCancellation,
) -> (u16, Vec<u8>) {
    if !request.body.is_empty() {
        return (
            400,
            json_error("game_information_content_manifest_body_forbidden"),
        );
    }
    let Some(correlation) = request
        .headers
        .get(manifest::CORRELATION_HEADER)
        .map(String::as_str)
    else {
        return (
            400,
            json_error("game_information_content_manifest_correlation_required"),
        );
    };
    if !manifest::correlation_identity(correlation) {
        return (
            400,
            json_error("game_information_content_manifest_correlation_invalid"),
        );
    }
    let authority = service.game_information_authority();
    let response = match service.forward_mod_with_limit_detailed_timeout_cancelable(
        "GET",
        GameInformationRoute::ContentManifest.downstream_path(),
        &[],
        Some(correlation),
        manifest::MAX_MANIFEST_BYTES,
        service.game_information_exchange_timeout,
        cancellation,
    ) {
        Ok(response) => response,
        // A declared body beyond the admitted bound is refused from the declaration alone, so no
        // producer byte can be mistaken for the beginning of a shorter manifest.
        Err(ReadError::Oversized) => return oversized(correlation),
        Err(ReadError::Cancelled) => {
            return super::errors::game_information_transport_error(ReadError::Cancelled);
        }
        Err(ReadError::Timeout) => {
            return (504, json_error("game_information_content_manifest_timeout"));
        }
        Err(ReadError::Malformed) => return invalid_response(),
        Err(ReadError::Unavailable) => {
            return (
                503,
                json_error("game_information_content_manifest_unavailable"),
            );
        }
    };
    if let Err(error) = service.check_lease(request) {
        return error;
    }
    if service.game_information_authority() != authority {
        return scope_rejected();
    }
    let pinned = manifest::pinned_revision(&service.config.game_information_content_manifest_id);
    match manifest::validate_response(&response.body, &request.headers, pinned, response.status) {
        Ok(()) => (response.status, response.body),
        Err(manifest::Error::Oversized) => oversized(correlation),
        Err(manifest::Error::Revision) => scope_rejected(),
        Err(manifest::Error::Invalid) => invalid_response(),
    }
}

fn oversized(correlation: &str) -> (u16, Vec<u8>) {
    (413, manifest::refusal_envelope(correlation))
}

fn scope_rejected() -> (u16, Vec<u8>) {
    (
        409,
        json_error("game_information_content_manifest_scope_rejected"),
    )
}

fn invalid_response() -> (u16, Vec<u8>) {
    (
        502,
        json_error("game_information_content_manifest_response_invalid"),
    )
}
