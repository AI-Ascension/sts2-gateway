// SPDX-License-Identifier: MIT

use super::{HttpRequest, RuntimeService, json_error};
use sts2_gateway::validate_exact_checkpoint_reference;

const MAX_REFERENCE_RESPONSE_BYTES: usize = 8192;

impl RuntimeService {
    pub(super) fn checkpoint_reference_request(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if self.shutdown_requested || self.lease_revoked {
            return (409, json_error("lease_not_active"));
        }
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.body.is_empty() {
            return (400, json_error("checkpoint_reference_body_forbidden"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit(
            "GET",
            "/api/checkpoint/v1/reference",
            &[],
            correlation,
            MAX_REFERENCE_RESPONSE_BYTES,
        ) {
            Ok(response) => response,
            Err(502) => return invalid_response(),
            Err(_) => return unavailable(),
        };
        if matches!(response.status, 404 | 501 | 503) {
            return unavailable();
        }
        if response.status != 200 || !valid_response(request, &response.body) {
            return invalid_response();
        }
        (200, response.body)
    }
}

fn valid_response(request: &HttpRequest, bytes: &[u8]) -> bool {
    let Ok(value) = super::super::strict_json::parse(bytes) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 8 || value["schema"] != "ascension.checkpoint_reference_response.v1" {
        return false;
    }
    for (field, header) in [
        ("instance_id", "x-sts2-instance-id"),
        ("caller_id", "x-sts2-caller-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ] {
        if value[field].as_str() != request.headers.get(header).map(String::as_str) {
            return false;
        }
    }
    let epoch = request
        .headers
        .get("x-sts2-lease-epoch")
        .and_then(|v| v.parse::<u64>().ok());
    value["lease_epoch"].as_u64() == epoch
        && validate_exact_checkpoint_reference(&value["reference"]).is_ok()
}

fn unavailable() -> (u16, Vec<u8>) {
    (503, json_error("checkpoint_reference_unavailable"))
}

fn invalid_response() -> (u16, Vec<u8>) {
    (502, json_error("checkpoint_reference_response_invalid"))
}
