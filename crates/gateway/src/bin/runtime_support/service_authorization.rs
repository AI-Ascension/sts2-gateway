// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn request_rejection(
    request: &HttpRequest,
    auth_policy: &AuthPolicy,
    instance_id: &str,
    recovery_enabled: bool,
) -> Option<(u16, Vec<u8>)> {
    if !headers_are_allowed(&request.headers) {
        return Some((400, json_error("unsupported_header")));
    }
    let provided = request.headers.get("authorization").map(String::as_str);
    let scope = required_scope(request, instance_id);
    let result = if recovery_enabled && is_recovery_path(request) {
        auth_policy.authorize_recovery(provided, scope)
    } else {
        auth_policy.authorize(provided, scope)
    };
    match result {
        Ok(()) => None,
        Err(AuthFailure::Missing | AuthFailure::Invalid) => Some((401, json_error("unauthorized"))),
        Err(AuthFailure::Expired) => Some((401, json_error("token_expired"))),
        Err(AuthFailure::Scope) => Some((403, json_error("insufficient_scope"))),
    }
}

pub(super) fn is_recovery_path(request: &HttpRequest) -> bool {
    request.method == "POST"
        && matches!(
            request.path.as_str(),
            "/v1/recovery/bootstrap"
                | "/v1/recovery/host-fence"
                | "/v1/recovery/lease/acquire"
                | "/v1/recovery/lease/renew"
                | "/v1/recovery/lease/revoke"
                | "/v1/recovery/operation/intent"
                | "/v1/recovery/operation/dispatch"
                | "/v1/recovery/operation/lookup"
                | "/v1/recovery/operation/reconcile"
        )
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
    if let Some(route) = RuntimeV4ExpertRoute::parse(&request.method, &request.path, instance_id) {
        return if route.is_dispatch() {
            AuthScope::Mutate
        } else {
            AuthScope::Read
        };
    }
    if let Some(route) =
        RuntimeV4ExpertRestActionRoute::parse(&request.method, &request.path, instance_id)
    {
        return if route.is_dispatch() {
            AuthScope::Mutate
        } else {
            AuthScope::Read
        };
    }
    if RuntimeMapRoute::parse(&request.method, &request.path, instance_id).is_some() {
        return AuthScope::Read;
    }
    let seeded_start_path = format!("/v2/instances/{instance_id}/seeded-run");
    let seeded_operation_prefix = format!("/v2/instances/{instance_id}/seeded-operations/");
    if request.method == "POST" && request.path == seeded_start_path {
        return AuthScope::Mutate;
    }
    if request.method == "GET" && request.path.starts_with(&seeded_operation_prefix) {
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
    let host_fence_path = "/v1/recovery/host-fence";
    let bootstrap_path = "/v1/recovery/bootstrap";
    let lease_acquire_path = "/v1/recovery/lease/acquire";
    let lease_renew_path = "/v1/recovery/lease/renew";
    let lease_revoke_path = "/v1/recovery/lease/revoke";
    let operation_intent_path = "/v1/recovery/operation/intent";
    let operation_dispatch_path = "/v1/recovery/operation/dispatch";
    let operation_lookup_path = "/v1/recovery/operation/lookup";
    let operation_reconcile_path = "/v1/recovery/operation/reconcile";
    if request.method == "POST" {
        if request.path == operation_lookup_path {
            return AuthScope::Read;
        }
        if request.path == operation_intent_path || request.path == operation_dispatch_path {
            return AuthScope::Mutate;
        }
        if request.path == bootstrap_path
            || request.path == host_fence_path
            || request.path == lease_acquire_path
            || request.path == lease_renew_path
            || request.path == lease_revoke_path
            || request.path == operation_reconcile_path
        {
            return AuthScope::Control;
        }
    }
    if request.method == "POST"
        && (request.path == allocate_path
            || request.path == release_path
            || request.path == shutdown_path
            || request.path == coop_report_path
            || request.path == host_fence_path)
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
                | "x-sts2-recovery-capability"
        )
    })
}
