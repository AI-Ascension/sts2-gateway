// SPDX-License-Identifier: MIT

use super::*;

use super::authorization::first_rejected_header;
use super::test_support::*;

#[test]
fn bearer_authentication_requires_an_exact_value() {
    let policy = AuthPolicy::test_all("gateway-token");
    assert!(
        policy
            .authorize(Some("Bearer gateway-token"), AuthScope::Read)
            .is_ok()
    );
    for value in [
        None,
        Some(""),
        Some("gateway-token"),
        Some("Bearer wrong-token"),
        Some("bearer gateway-token"),
        Some("Bearer gateway-token "),
        Some("Bearer gateway-token\n"),
    ] {
        assert!(policy.authorize(value, AuthScope::Read).is_err());
    }
}

#[test]
fn missing_and_wrong_authentication_fail_before_lease_processing() -> Result<(), String> {
    let mut service = test_service()?;
    let mut missing = authenticated_request("/health/ready");
    missing.headers.remove("authorization");
    assert_eq!(service.handle_request(&missing).0, 401);

    let mut wrong = authenticated_request("/health/ready");
    wrong
        .headers
        .insert(String::from("authorization"), String::from("Bearer wrong"));
    assert_eq!(service.handle_request(&wrong).0, 401);
    Ok(())
}

#[test]
fn expired_and_under_scoped_credentials_fail_at_the_gateway_boundary() -> Result<(), String> {
    let mut expired = test_service()?;
    expired.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", Some(1), None, "read,mutate,control")?;
    let request = authenticated_request("/v2/instances/instance-1/state");
    let (status, body) = expired.handle_request(&request);
    assert_eq!(status, 401);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "token_expired");

    let mut scoped = test_service()?;
    scoped.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
    let mut action = authenticated_request("/v2/instances/instance-1/action");
    action.method = String::from("POST");
    action.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    let (status, body) = scoped.handle_request(&action);
    assert_eq!(status, 403);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "insufficient_scope");
    Ok(())
}

#[test]
fn previous_credential_is_accepted_during_gateway_rotation() -> Result<(), String> {
    let mut service = test_service()?;
    service.config.auth_policy = AuthPolicy::test_with_previous(
        "new-gateway-token",
        None,
        Some(("gateway-token", None)),
        "read,mutate,control",
    )?;
    let request = authenticated_request("/v2/instances/instance-1/state");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
    Ok(())
}

#[test]
fn wrong_instance_is_rejected_even_with_valid_authentication() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v2/instances/other/state");
    request
        .headers
        .insert(String::from("x-sts2-instance-id"), String::from("other"));
    assert_eq!(service.handle_request(&request).0, 404);

    let request = authenticated_request("/v2/instances/instance-1/state");
    let mut wrong_fence = request;
    wrong_fence
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("2"));
    assert_eq!(service.handle_request(&wrong_fence).0, 409);
    Ok(())
}

#[test]
fn wrong_mcp_session_is_rejected_before_downstream_forwarding() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v2/instances/instance-1/state");
    request.headers.insert(
        String::from("x-mcp-session-id"),
        String::from("other-mcp-session"),
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 409);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "lease_fence_rejected");
    Ok(())
}

#[test]
fn v3_routes_enforce_scopes_before_envelope_and_lease_processing() -> Result<(), String> {
    let routes = [
        ("GET", "state", "read"),
        ("GET", "legal-actions", "read"),
        ("POST", "action", "mutate"),
        ("POST", "wait", "read"),
        ("GET", "reobserve", "read"),
        ("POST", "recover", "control"),
    ];
    for (method, suffix, required) in routes {
        for scope in ["read", "mutate", "control"] {
            let mut service = test_service()?;
            service.config.auth_policy =
                AuthPolicy::test_with_previous("gateway-token", None, None, scope)?;
            let mut request = authenticated_request(&format!("/v3/instances/instance-1/{suffix}"));
            request.method = method.to_owned();
            request
                .headers
                .insert("content-type".to_owned(), "application/json".to_owned());
            request.body = b"{}".to_vec();
            let expected = if scope == required { 400 } else { 403 };
            assert_eq!(
                service.handle_request(&request).0,
                expected,
                "{suffix}: {scope}"
            );
        }
    }
    Ok(())
}

#[test]
fn host_fence_route_requires_control_scope_before_frame_or_lease_processing() -> Result<(), String>
{
    for scope in ["read", "mutate", "control"] {
        let mut service = test_service()?;
        service.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", None, None, scope)?;
        let mut request = authenticated_request("/v1/recovery/host-fence");
        request.method = String::from("POST");
        request.headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        request.body = b"{}".to_vec();
        request.headers.remove("x-sts2-lease-id");
        request.headers.remove("x-sts2-lease-epoch");
        assert_eq!(
            service.handle_request(&request).0,
            if scope == "control" { 400 } else { 403 },
            "{scope}"
        );
    }
    Ok(())
}

#[test]
fn v3_routes_keep_the_configured_mcp_session_fence() -> Result<(), String> {
    for (method, suffix) in [
        ("GET", "state"),
        ("GET", "legal-actions"),
        ("POST", "action"),
        ("POST", "wait"),
        ("GET", "reobserve"),
        ("POST", "recover"),
    ] {
        let mut service = test_service()?;
        let mut request = authenticated_request(&format!("/v3/instances/instance-1/{suffix}"));
        request.method = method.to_owned();
        request
            .headers
            .insert("x-mcp-session-id".to_owned(), "other-session".to_owned());
        assert_eq!(service.handle_request(&request).0, 409, "{suffix}");
    }
    Ok(())
}

#[test]
fn a_rejected_header_is_named_in_the_refusal_so_a_recurrence_is_decidable()
-> Result<(), String> {
    // The refusal is unattributable without the name: `{"error_code":
    // "unsupported_header"}` does not say which header was refused, so an
    // intermittent refusal can only be adjudicated by re-running
    // (AI-Ascension/sts2-harness#541).
    let mut service = test_service()?;
    let mut request = authenticated_request("/health/ready");
    request
        .headers
        .insert("accept".to_owned(), "application/json".to_owned());
    let (status, bytes) = service.handle_request(&request);
    assert_eq!(status, 400);
    let body = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "unsupported_header");
    assert_eq!(value["rejected_header"], "accept");
    Ok(())
}

#[test]
fn a_refusal_never_echoes_a_header_value() -> Result<(), String> {
    // The value may be a credential, so only the name crosses the boundary.
    let secret = "Bearer super-secret-token-value";
    let mut service = test_service()?;
    let mut request = authenticated_request("/health/ready");
    request
        .headers
        .insert("x-unlisted-credential".to_owned(), secret.to_owned());
    let (status, bytes) = service.handle_request(&request);
    assert_eq!(status, 400);
    let body = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    assert!(
        !body.contains(secret),
        "a header value must never be echoed: {body}"
    );
    assert!(
        !body.contains("super-secret-token-value"),
        "no fragment of the value may survive: {body}"
    );
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["rejected_header"], "x-unlisted-credential");
    Ok(())
}

#[test]
fn an_admitted_header_set_is_not_refused() -> Result<(), String> {
    // The remediation is additive: a request whose headers are all on the
    // allow-list must not be refused, and `error_code` must be unchanged for a
    // consumer that matches on it.
    let request = authenticated_request("/health/ready");
    assert_eq!(first_rejected_header(&request.headers), None);
    Ok(())
}

#[test]
fn the_rejection_precedes_route_resolution_for_every_path() -> Result<(), String> {
    // `request_rejection` runs before route parsing, so the refusal is the same
    // on any path. Pin that, because it is what makes an unattributed refusal
    // ambiguous between hops.
    let mut service = test_service()?;
    for path in ["/health/ready", "/v1/no-such-route", "/workflow-runs"] {
        let mut request = authenticated_request(path);
        request
            .headers
            .insert("accept".to_owned(), "application/json".to_owned());
        let (status, bytes) = service.handle_request(&request);
        assert_eq!(status, 400, "{path}");
        let body = String::from_utf8(bytes).map_err(|error| error.to_string())?;
        assert!(body.contains("unsupported_header"), "{path}: {body}");
    }
    Ok(())
}

#[test]
fn a_refusal_names_one_header_deterministically() -> Result<(), String> {
    // A request can carry several unlisted headers. The refusal must be
    // reproducible for the same request, so the reported name is taken from
    // the request's own `BTreeMap` order rather than from an unordered scan.
    let mut service = test_service()?;
    let mut request = authenticated_request("/health/ready");
    request
        .headers
        .insert("x-sts2-unlisted-a".to_owned(), String::from("first"));
    request
        .headers
        .insert("x-sts2-unlisted-b".to_owned(), String::from("second"));
    let (status, bytes) = service.handle_request(&request);
    assert_eq!(status, 400);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "unsupported_header");
    assert_eq!(value["rejected_header"], "x-sts2-unlisted-a");
    Ok(())
}

#[test]
fn an_allowed_header_is_never_named_as_the_rejected_one() -> Result<(), String> {
    // `authorization` is on the allow-list even though it carries a
    // credential, so the refusal must not be able to promote an admitted
    // header into the reported name.
    let mut service = test_service()?;
    let mut request = authenticated_request("/health/ready");
    request
        .headers
        .insert("x-sts2-unlisted".to_owned(), String::from("value"));
    let (_, bytes) = service.handle_request(&request);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    assert_ne!(value["rejected_header"], "authorization");
    assert_eq!(value["rejected_header"], "x-sts2-unlisted");
    Ok(())
}
