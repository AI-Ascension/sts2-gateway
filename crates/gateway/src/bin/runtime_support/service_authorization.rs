// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn request_rejection(
    request: &HttpRequest,
    auth_policy: &AuthPolicy,
    instance_id: &str,
) -> Option<(u16, Vec<u8>)> {
    if !headers_are_allowed(&request.headers) {
        return Some((400, json_error("unsupported_header")));
    }
    match auth_policy.authorize(
        request.headers.get("authorization").map(String::as_str),
        required_scope(request, instance_id),
    ) {
        Ok(()) => None,
        Err(AuthFailure::Missing | AuthFailure::Invalid) => Some((401, json_error("unauthorized"))),
        Err(AuthFailure::Expired) => Some((401, json_error("token_expired"))),
        Err(AuthFailure::Scope) => Some((403, json_error("insufficient_scope"))),
    }
}

pub(super) fn required_scope(request: &HttpRequest, instance_id: &str) -> AuthScope {
    if let Some(route) = RuntimeV3GameplayRoute::parse(&request.method, &request.path, instance_id)
    {
        return match route {
            RuntimeV3GameplayRoute::DispatchAction => AuthScope::Mutate,
            RuntimeV3GameplayRoute::Recover => AuthScope::Control,
            RuntimeV3GameplayRoute::State
            | RuntimeV3GameplayRoute::LegalActions
            | RuntimeV3GameplayRoute::WaitForTransition
            | RuntimeV3GameplayRoute::Reobserve => AuthScope::Read,
        };
    }
    if RuntimeMapRoute::parse(&request.method, &request.path, instance_id).is_some() {
        return AuthScope::Read;
    }
    let action_path = format!("/v2/instances/{instance_id}/action");
    let legacy_action_path = format!("/v1/instances/{instance_id}/action");
    if request.method == "POST"
        && (request.path == action_path || request.path == legacy_action_path)
    {
        return AuthScope::Mutate;
    }
    let allocate_path = "/v1/sessions/allocate";
    let release_path = format!("/v1/instances/{instance_id}/release");
    let shutdown_path = format!("/v2/instances/{instance_id}/shutdown");
    let coop_report_path = format!("/v1/instances/{instance_id}/coop/peer-report");
    if request.method == "POST"
        && (request.path == allocate_path
            || request.path == release_path
            || request.path == shutdown_path
            || request.path == coop_report_path)
    {
        return AuthScope::Control;
    }
    AuthScope::Read
}

pub(super) fn headers_are_allowed(headers: &BTreeMap<String, String>) -> bool {
    headers.keys().all(|name| {
        matches!(
            name.as_str(),
            "authorization"
                | "connection"
                | "content-length"
                | "content-type"
                | "host"
                | "x-mcp-request-id"
                | "x-mcp-session-id"
                | "x-sts2-instance-id"
                | "x-sts2-caller-id"
                | "x-sts2-session-id"
                | "x-sts2-lease-id"
                | "x-sts2-lease-epoch"
                | "x-sts2-correlation-id"
        )
    })
}
