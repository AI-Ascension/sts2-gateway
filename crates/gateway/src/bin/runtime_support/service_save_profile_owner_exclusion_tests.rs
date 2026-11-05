// SPDX-License-Identifier: MIT

//! Cross-owner source tests, not native process or game acceptance.

use super::*;
use super::test_support::authenticated_request;

fn create_request() -> HttpRequest {
    let mut request = authenticated_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
    );
    request.method = String::from("POST");
    request.headers.insert(
        String::from("x-mcp-request-id"),
        String::from("owner-exclusion-1"),
    );
    request
}

fn lifecycle_request(operation_id: u64, action: &str) -> HttpRequest {
    let mut request = authenticated_request(
        "/v1/instances/instance-1/process-lifecycle/operations",
    );
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = format!(
        r#"{{"operation_id":{operation_id},"authority_epoch":1,"action":{action}}}"#,
    )
    .into_bytes();
    request
}

fn assert_error(response: (u16, Vec<u8>), status: u16, code: &str) {
    assert_eq!(response.0, status);
    assert!(
        String::from_utf8_lossy(&response.1).contains(code),
        "expected {code}: {}",
        String::from_utf8_lossy(&response.1)
    );
}

#[test]
fn absent_owner_never_becomes_idle_authority() -> Result<(), String> {
    let mut service = process_lifecycle_tests::lifecycle_service_for_owner_exclusion()?;
    assert_error(
        service.handle_request(&create_request()),
        503,
        "save_profile_active_run_unavailable",
    );
    Ok(())
}

#[test]
fn retained_owner_overrides_idle_claim_until_stopped() -> Result<(), String> {
    let mut service = process_lifecycle_tests::lifecycle_service_for_owner_exclusion()?;
    // This source-only injection models a separate idle writer. An actual
    // production idle writer remains unconfigured until native qualification.
    service.save_profile_active_run = SaveProfileActiveRun {
        governed: Some(false),
    };
    assert_eq!(
        service.handle_request(&lifecycle_request(
            1,
            r#"{"kind":"launch_new","profile_id":7}"#,
        )).0,
        200
    );
    assert_error(
        service.handle_request(&create_request()),
        409,
        "save_profile_active_run",
    );
    assert_eq!(
        service.handle_request(&lifecycle_request(
            2,
            r#"{"kind":"stop","mode":"graceful"}"#,
        )).0,
        200
    );
    // An absent owner still cannot authorize writes: the synthetic idle
    // writer has no durable save-profile operation store or native witness.
    assert_error(
        service.handle_request(&create_request()),
        503,
        "save_profile_persistence_unavailable",
    );
    Ok(())
}
