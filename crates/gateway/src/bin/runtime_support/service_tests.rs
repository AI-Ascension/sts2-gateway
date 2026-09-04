// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::Value;
use sts2_gateway::{
    RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Observation,
};

use super::{HttpRequest, RuntimeConfig, RuntimeService, UnconfiguredRuntimeV2Forwarder};

fn test_service() -> Result<RuntimeService, String> {
    let config = RuntimeConfig {
        listen_address: String::from("127.0.0.1:15525"),
        mod_address: String::from("127.0.0.1:15526"),
        gateway_token: String::from("gateway-token"),
        mod_token: String::from("mod-token"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
    };
    let binding = RuntimeV2Binding::new(
        &config.instance_id,
        &config.session_id,
        &config.lease_id,
        config.lease_epoch,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
    )
    .map_err(|error| error.to_string())?;
    let runtime_v2 = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(8),
        binding,
        UnconfiguredRuntimeV2Forwarder,
    )
    .map_err(|error| error.to_string())?;
    Ok(RuntimeService {
        config,
        lease_active: true,
        lease_released: false,
        runtime_v2,
    })
}

fn authenticated_request(path: &str) -> HttpRequest {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("authorization"),
        String::from("Bearer gateway-token"),
    );
    headers.insert(
        String::from("x-sts2-instance-id"),
        String::from("instance-1"),
    );
    headers.insert(String::from("x-sts2-caller-id"), String::from("harness"));
    headers.insert(String::from("x-sts2-session-id"), String::from("session-1"));
    headers.insert(String::from("x-sts2-lease-id"), String::from("lease-1"));
    headers.insert(String::from("x-sts2-lease-epoch"), String::from("1"));
    headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr-state"),
    );
    HttpRequest {
        method: String::from("GET"),
        path: path.to_owned(),
        headers,
        body: Vec::new(),
    }
}

#[test]
fn state_route_returns_typed_request_and_explicit_unavailable_fallback() -> Result<(), String> {
    let mut service = test_service()?;
    let request = authenticated_request("/v2/instances/instance-1/state");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["status"], "unavailable");
    assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
    assert_eq!(value["reason"], "unconfigured_runtime_v2_forwarder");
    assert_eq!(value["request"]["kind"], "state_request");
    assert_eq!(value["request"]["instance_id"], "instance-1");
    assert_eq!(value["request"]["correlation_id"], "corr-state");
    Ok(())
}

#[test]
fn v2_gets_are_not_arbitrary_proxy_routes() -> Result<(), String> {
    let mut service = test_service()?;
    for path in [
        "/v2/instances/instance-1/state/extra",
        "/v2/instances/instance-1/not-a-proxy",
    ] {
        let (status, _) = service.handle_request(&authenticated_request(path));
        assert_eq!(status, 404, "unexpected route match for {path}");
    }
    Ok(())
}

#[test]
fn runtime_addresses_are_literal_loopback_only() {
    for address in ["127.0.0.1:15525", "[::1]:15526"] {
        assert!(super::service_config::validate_loopback_address(address).is_ok());
    }
    for address in [
        "0.0.0.0:15525",
        "[::]:15525",
        "192.0.2.1:15526",
        "localhost:15525",
        "example.com:15526",
        "127.0.0.1:0",
    ] {
        assert!(super::service_config::validate_loopback_address(address).is_err());
    }
}

#[test]
fn released_context_cannot_be_reactivated() -> Result<(), String> {
    let mut service = test_service()?;
    let mut release = authenticated_request("/v1/instances/instance-1/release");
    release.method = "POST".to_owned();
    assert_eq!(service.handle_request(&release).0, 200);
    let allocation =
        br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#;
    assert_eq!(service.allocate(allocation).0, 409);
    assert!(service.check_lease(&release).is_err());
    Ok(())
}

#[test]
fn allocation_rejects_duplicate_unknown_and_missing_members() -> Result<(), String> {
    for body in [
        br#"{"instance_id":"wrong","instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#.as_slice(),
        br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1","extra":true}"#,
        br#"{"instance_id":"instance-1","caller_id":"harness"}"#,
    ] {
        let mut service = test_service()?;
        service.lease_active = false;
        assert_eq!(service.allocate(body).0, 400);
        assert!(!service.lease_active);
    }
    let mut service = test_service()?;
    service.lease_active = false;
    assert_eq!(
        service
            .allocate(br#"{"instance_id":"wrong","caller_id":"harness","session_id":"session-1"}"#)
            .0,
        409
    );
    assert!(!service.lease_active);
    assert_eq!(
        service
            .allocate(
                br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#
            )
            .0,
        200
    );
    assert!(service.lease_active);
    Ok(())
}
