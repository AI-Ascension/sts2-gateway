// SPDX-License-Identifier: MIT

//! Fixed authenticated exact-restore routes. The Gateway binds every phase to its live owner
//! before forwarding the neutral frame to the fixed native path.

use serde_json::{Value, json};
use sts2_gateway::RecoveryContinuationOwnerState;

use self::validation::{canonical_bytes, valid_neutral_schema, valid_wrapper_schema};
use super::super::{HttpRequest, RuntimeService, json_error};

const CONTRACT: &str = "sts2-exact-restore-gateway-v1";
const WRAPPER_SCHEMA_DIGEST: &str =
    "0b181dc30524c8b14dea73e490da55538f2d57fe87bf58ed9fe33223406a7d89";
const NEUTRAL_CONTRACT: &str = "sts2-exact-restore-v1";
const NEUTRAL_SCHEMA_DIGEST: &str =
    "2289d888c33eac46873408303c4423eab762e3f7bd6132ae8ae88d0d3b1858e4";
const MAX_FRAME_BYTES: usize = 16 * 1024;
const CAPABILITY: &str = "exact_restore";

#[path = "service_exact_restore_validation.rs"]
mod validation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Route {
    Begin,
    Chunk,
    Finish,
    Commit,
    Lookup,
}

impl Route {
    fn parse_path(path: &str) -> Option<Self> {
        match path {
            "/v1/exact-restore/begin" => Some(Self::Begin),
            "/v1/exact-restore/chunk" => Some(Self::Chunk),
            "/v1/exact-restore/finish" => Some(Self::Finish),
            "/v1/exact-restore/commit" => Some(Self::Commit),
            "/v1/exact-restore/lookup" => Some(Self::Lookup),
            _ => None,
        }
    }

    const fn request_kind(self) -> &'static str {
        match self {
            Self::Begin => "exact_restore_begin_request",
            Self::Chunk => "exact_restore_chunk_request",
            Self::Finish => "exact_restore_finish_blob_request",
            Self::Commit => "exact_restore_commit_request",
            Self::Lookup => "exact_restore_lookup_request",
        }
    }

    const fn response_kind(self) -> &'static str {
        match self {
            Self::Begin => "exact_restore_begin_response",
            Self::Chunk => "exact_restore_chunk_response",
            Self::Finish => "exact_restore_finish_blob_response",
            Self::Commit => "exact_restore_commit_response",
            Self::Lookup => "exact_restore_lookup_response",
        }
    }

    const fn path(self) -> &'static str {
        match self {
            Self::Begin => "/v1/exact-restore/begin",
            Self::Chunk => "/v1/exact-restore/chunk",
            Self::Finish => "/v1/exact-restore/finish",
            Self::Commit => "/v1/exact-restore/commit",
            Self::Lookup => "/v1/exact-restore/lookup",
        }
    }

    pub(super) fn parse(method: &str, path: &str) -> Option<Self> {
        (method == "POST").then(|| Self::parse_path(path)).flatten()
    }
}

struct RequestFrame {
    principal: String,
    message_id: String,
    correlation_id: String,
    operation_id: String,
    expected_owner: Value,
    request_digest: String,
    neutral_bytes: Vec<u8>,
}

pub(super) fn handle(
    service: &mut RuntimeService,
    request: &HttpRequest,
    route: Route,
) -> (u16, Vec<u8>) {
    if service.recovery.is_none() {
        return (503, json_error("exact_restore_owner_unavailable"));
    }
    if request
        .headers
        .get("x-sts2-recovery-capability")
        .map(String::as_str)
        != Some(CAPABILITY)
    {
        return (403, json_error("exact_restore_capability_forbidden"));
    }
    if let Err(error) = service.check_lease(request) {
        return error;
    }
    if !request.content_type_is_json() || request.body.is_empty() {
        return (400, json_error("exact_restore_frame_required"));
    }
    let frame = match parse_request(service, request, route) {
        Ok(frame) => frame,
        Err(error) => return error,
    };
    if let Err(error) = validate_live_owner(service, request, &frame.expected_owner) {
        return error;
    }

    let response = match service.forward_mod_with_limit(
        "POST",
        route.path(),
        &frame.neutral_bytes,
        Some(&frame.correlation_id),
        MAX_FRAME_BYTES,
    ) {
        Ok(response) => response,
        Err(502) => return (502, json_error("exact_restore_response_invalid")),
        Err(_) => return (503, json_error("exact_restore_downstream_unavailable")),
    };
    let neutral_response = match validate_response(&response.body, route, &frame) {
        Some(value) => value,
        None => return (502, json_error("exact_restore_response_invalid")),
    };
    if let Err(error) = service.check_lease(request) {
        return error;
    }
    if let Err(error) = validate_live_owner(service, request, &frame.expected_owner) {
        return error;
    }
    match wrap_response(&frame, &neutral_response) {
        Some(bytes) => (response.status, bytes),
        None => (502, json_error("exact_restore_response_invalid")),
    }
}

fn parse_request(
    service: &RuntimeService,
    request: &HttpRequest,
    route: Route,
) -> Result<RequestFrame, (u16, Vec<u8>)> {
    if request.body.len() > MAX_FRAME_BYTES {
        return Err((413, json_error("exact_restore_frame_oversized")));
    }
    let wrapper = super::super::super::strict_json::parse(&request.body)
        .map_err(|_| (400, json_error("exact_restore_frame_invalid")))?;
    if !canonical_bytes(&wrapper, &request.body) || !valid_wrapper_schema(&wrapper) {
        return Err((400, json_error("exact_restore_frame_invalid")));
    }
    if wrapper["contract"] != CONTRACT
        || wrapper["schema_digest"] != WRAPPER_SCHEMA_DIGEST
        || wrapper["kind"] != "exact_restore_request"
        || wrapper["actor"]["role"] != "harness"
        || wrapper["auth"]["capability"] != CAPABILITY
        || !wrapper["auth"]["proof"].is_null()
    {
        return Err((400, json_error("exact_restore_contract_unsupported")));
    }
    let principal = wrapper["actor"]["principal_id"]
        .as_str()
        .filter(|principal| *principal == service.config.caller_id)
        .ok_or_else(|| (403, json_error("exact_restore_principal_rejected")))?
        .to_owned();
    if wrapper["auth"]["principal_id"].as_str() != Some(principal.as_str()) {
        return Err((403, json_error("exact_restore_principal_rejected")));
    }
    let frame = &wrapper["payload"]["frame"];
    let neutral_bytes =
        serde_json::to_vec(frame).map_err(|_| (400, json_error("exact_restore_frame_invalid")))?;
    if neutral_bytes.len() > MAX_FRAME_BYTES
        || frame["contract"] != NEUTRAL_CONTRACT
        || frame["schema_digest"] != NEUTRAL_SCHEMA_DIGEST
        || frame["kind"] != route.request_kind()
        || !valid_neutral_schema(frame)
    {
        return Err((400, json_error("exact_restore_frame_invalid")));
    }
    let message_id = frame["message_id"]
        .as_str()
        .ok_or_else(|| (400, json_error("exact_restore_frame_invalid")))?
        .to_owned();
    let correlation_id = frame["correlation_id"]
        .as_str()
        .ok_or_else(|| (400, json_error("exact_restore_frame_invalid")))?
        .to_owned();
    if wrapper["message_id"].as_str() != Some(message_id.as_str())
        || wrapper["correlation_id"].as_str() != Some(correlation_id.as_str())
        || request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(correlation_id.as_str())
    {
        return Err((400, json_error("exact_restore_correlation_rejected")));
    }
    let payload = &frame["payload"];
    let operation_id = payload["operation_id"]
        .as_str()
        .ok_or_else(|| (400, json_error("exact_restore_frame_invalid")))?
        .to_owned();
    let expected_owner = payload["expected_owner"].clone();
    let request_digest = format!("sha256:{}", sts2_gateway::sha256_hex(&neutral_bytes));
    Ok(RequestFrame {
        principal,
        message_id,
        correlation_id,
        operation_id,
        expected_owner,
        request_digest,
        neutral_bytes,
    })
}

fn validate_live_owner(
    service: &mut RuntimeService,
    request: &HttpRequest,
    expected_owner: &Value,
) -> Result<(), (u16, Vec<u8>)> {
    let snapshot = super::recovery_owner::read_owner_snapshot(service)
        .map_err(super::super::recovery_wire::recovery_store_error)?;
    if snapshot.state != RecoveryContinuationOwnerState::Available {
        return Err(super::recovery_owner::owner_state_error(snapshot.state));
    }
    let current_owner = snapshot
        .owner
        .as_ref()
        .and_then(|owner| serde_json::to_value(owner).ok());
    if current_owner.as_ref() != Some(expected_owner) {
        return Err((409, json_error("exact_restore_owner_fence_mismatch")));
    }
    service.check_lease(request)
}

fn validate_response(bytes: &[u8], route: Route, request: &RequestFrame) -> Option<Value> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return None;
    }
    let frame = super::super::super::strict_json::parse(bytes).ok()?;
    if !canonical_bytes(&frame, bytes)
        || !valid_neutral_schema(&frame)
        || frame["contract"] != NEUTRAL_CONTRACT
        || frame["schema_digest"] != NEUTRAL_SCHEMA_DIGEST
    {
        return None;
    }
    let kind = frame["kind"].as_str()?;
    if kind != route.response_kind() && kind != "exact_restore_error_response" {
        return None;
    }
    let payload = &frame["payload"];
    if frame["correlation_id"].as_str() != Some(request.message_id.as_str())
        || payload["operation_id"].as_str() != Some(request.operation_id.as_str())
        || payload["expected_owner"] != request.expected_owner
        || payload["request_digest"].as_str() != Some(request.request_digest.as_str())
    {
        return None;
    }
    Some(frame)
}

fn wrap_response(request: &RequestFrame, frame: &Value) -> Option<Vec<u8>> {
    let response = json!({
        "contract": CONTRACT,
        "schema_digest": WRAPPER_SCHEMA_DIGEST,
        "message_id": frame["message_id"],
        "correlation_id": frame["correlation_id"],
        "actor": {
            "principal_id": request.principal,
            "role": "gateway"
        },
        "auth": {
            "principal_id": request.principal,
            "capability": CAPABILITY,
            "proof": null
        },
        "kind": "exact_restore_response",
        "payload": {"frame": frame}
    });
    let bytes = serde_json::to_vec(&response).ok()?;
    (bytes.len() <= MAX_FRAME_BYTES && valid_wrapper_schema(&response)).then_some(bytes)
}

#[cfg(test)]
#[path = "service_exact_restore_tests.rs"]
mod tests;
