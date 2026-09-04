// SPDX-License-Identifier: MIT

use super::*;

use super::test_support::*;

#[test]
fn v3_action_requires_mutate_scope() -> Result<(), String> {
    let mut service = test_service()?;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
    let mut request = authenticated_request("/v3/instances/instance-1/action");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = v3_action_body();
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 403);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "insufficient_scope");
    Ok(())
}

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
