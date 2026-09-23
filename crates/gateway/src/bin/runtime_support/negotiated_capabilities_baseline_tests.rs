// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::net::TcpListener;

use super::super::super::auth::AuthPolicy;
use super::super::test_support::{serve_http_sequence, test_service};
use super::tests::{install_capabilities, request};

const RUNTIME_V3_STATE_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
));

fn runtime_v3_state_response_for(
    request: &super::super::super::http::HttpRequest,
) -> Result<Vec<u8>, String> {
    let correlation = request
        .headers
        .get("x-sts2-correlation-id")
        .ok_or("correlation header missing")?;
    let mut response: Value =
        serde_json::from_slice(RUNTIME_V3_STATE_RESPONSE).map_err(|error| error.to_string())?;
    response["correlation_id"] = Value::String(correlation.clone());
    serde_json::to_vec(&response).map_err(|error| error.to_string())
}

#[test]
fn route_advertises_runtime_v3_only_after_a_validated_configured_state_probe() -> Result<(), String>
{
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request = request();
    let producer = serve_http_sequence(
        listener,
        vec![(200, runtime_v3_state_response_for(&request)?)],
    );
    let mut service = test_service()?;
    service.config.mod_address = address;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("gateway-token", None, None, "read,mutate,control")?;
    install_capabilities(&mut service);

    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["runtime_v3_baseline_witness"],
        json!({
            "profile":"runtime-v3-gameplay",
            "schema_digest":"daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b",
            "configured_state_probe":true,
            "recovery_kinds":["reobserve","reconcile"]
        })
    );
    let operations = value["offers"]
        .as_array()
        .ok_or("offers missing")?
        .iter()
        .filter_map(|offer| offer["operation"].as_str())
        .filter(|operation| operation.starts_with("runtime_v3."))
        .collect::<Vec<_>>();
    assert_eq!(
        operations,
        vec![
            "runtime_v3.state",
            "runtime_v3.legal_actions",
            "runtime_v3.dispatch_action",
            "runtime_v3.wait",
            "runtime_v3.reobserve",
            "runtime_v3.recover"
        ]
    );
    let forwarded = producer
        .join()
        .map_err(|_| String::from("runtime-v3 producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].method, "GET");
    assert_eq!(forwarded[0].path, "/api/v3/runtime/state");
    Ok(())
}

#[test]
fn route_omits_runtime_v3_when_the_state_probe_is_not_a_validated_success() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_http_sequence(listener, vec![(503, b"{}".to_vec())]);
    let mut service = test_service()?;
    service.config.mod_address = address;
    install_capabilities(&mut service);

    let (status, body) = service.handle_request(&request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(value["runtime_v3_baseline_witness"].is_null());
    assert!(
        value["offers"]
            .as_array()
            .is_some_and(|offers| offers.iter().all(|offer| {
                !offer["operation"]
                    .as_str()
                    .is_some_and(|operation| operation.starts_with("runtime_v3."))
            }))
    );
    let forwarded = producer
        .join()
        .map_err(|_| String::from("runtime-v3 producer panicked"))??;
    assert_eq!(forwarded.len(), 1);
    Ok(())
}
