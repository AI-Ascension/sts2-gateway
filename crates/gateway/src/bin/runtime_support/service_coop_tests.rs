// SPDX-License-Identifier: MIT

use super::test_support::{authenticated_request, test_service};
use super::*;

fn configured() -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    service.coop_reports = Some(CoopReports::from_roster(
        r#"[{"peer_id":"local-1","role":"local"},{"peer_id":"ally-1","role":"ally"}]"#,
    )?);
    Ok(service)
}

fn report_request(peer: &str, generation: u64) -> HttpRequest {
    let mut request = authenticated_request("/v1/instances/instance-1/coop/peer-report");
    request.method = "POST".to_owned();
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = json_bytes(&json!({"peer_id":peer,"generation":generation,"connected":true}));
    request
}

#[test]
fn real_service_routes_report_and_emit_complete_pinned_schema_without_downstream()
-> Result<(), Box<dyn std::error::Error>> {
    let mut service = configured()?;
    // Port 1 is deliberately unavailable: these routes must not contact the game.
    let read = authenticated_request("/v1/instances/instance-1/coop/synchronization");
    let initial = service.handle_request(&read);
    assert_eq!(initial.0, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&initial.1)?["synchronization"]["status"],
        "disconnected"
    );
    for peer in ["local-1", "ally-1"] {
        assert_eq!(service.handle_request(&report_request(peer, 4)).0, 200);
    }
    let (status, bytes) = service.handle_request(&read);
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(value["generation"], 4);
    assert_eq!(value["source"], "gateway_peer_reports");
    assert_eq!(value["synchronization"]["status"], "synchronized");
    let schema: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/coop-synchronization-v1/schema.json"
    ))?;
    assert!(
        jsonschema::draft202012::options()
            .build(&schema)?
            .is_valid(&value)
    );
    let mut golden: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/coop-synchronization-v1/golden/synchronized.json"
    ))?;
    golden["correlation_id"] = json!("corr-state");
    assert_eq!(value, golden);
    Ok(())
}

#[test]
fn report_requires_control_scope_and_every_route_requires_exact_active_lease() -> Result<(), String>
{
    let mut service = configured()?;
    let read = authenticated_request("/v1/instances/instance-1/coop/synchronization");
    let report = report_request("local-1", 4);
    assert!(matches!(
        authorization::required_scope(&read, "instance-1"),
        AuthScope::Read
    ));
    assert!(matches!(
        authorization::required_scope(&report, "instance-1"),
        AuthScope::Control
    ));
    for original in [&read, &report] {
        for header in [
            "x-sts2-instance-id",
            "x-sts2-caller-id",
            "x-sts2-session-id",
            "x-mcp-session-id",
            "x-sts2-lease-id",
            "x-sts2-lease-epoch",
        ] {
            let mut request = HttpRequest {
                method: original.method.clone(),
                path: original.path.clone(),
                headers: original.headers.clone(),
                body: original.body.clone(),
            };
            request
                .headers
                .insert(header.to_owned(), "foreign".to_owned());
            assert_eq!(service.handle_request(&request).0, 409, "{header}");
        }
    }
    service.lease_active = false;
    assert_eq!(service.handle_request(&read).0, 409);
    assert_eq!(service.handle_request(&report).0, 409);
    Ok(())
}

#[test]
fn unconfigured_malformed_unknown_peer_and_regression_refuse() -> Result<(), String> {
    let read = authenticated_request("/v1/instances/instance-1/coop/synchronization");
    assert_eq!(test_service()?.handle_request(&read).0, 503);
    let mut service = configured()?;
    assert_eq!(service.handle_request(&report_request("foreign", 4)).0, 409);
    assert_eq!(service.handle_request(&report_request("local-1", 4)).0, 200);
    assert_eq!(service.handle_request(&report_request("local-1", 3)).0, 409);
    let mut request = report_request("local-1", 4);
    request.body = br#"{"peer_id":"local-1","generation":4,"connected":true,"extra":0}"#.to_vec();
    assert_eq!(service.handle_request(&request).0, 400);
    request.path = service.coop_synchronization_path();
    request.method = "GET".to_owned();
    assert_eq!(service.handle_request(&request).0, 400);
    request.body.clear();
    request.path.push_str("/extra");
    assert_eq!(service.handle_request(&request).0, 404);
    Ok(())
}

#[test]
fn read_credentials_cannot_submit_reports_and_missing_tokens_cannot_read() -> Result<(), String> {
    let mut service = configured()?;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
    let mut read = authenticated_request("/v1/instances/instance-1/coop/synchronization");
    assert_eq!(service.handle_request(&read).0, 200);
    assert_eq!(service.handle_request(&report_request("local-1", 4)).0, 403);
    read.headers.remove("authorization");
    assert_eq!(service.handle_request(&read).0, 401);
    Ok(())
}
