// SPDX-License-Identifier: MIT

use super::super::game_information_lookup_binding::RESPONSE_LIMIT_BYTES;
use super::test_support::{authenticated_request, serve_http_sequence, test_service};
use super::*;
use serde_json::{Value, json};
use std::io::ErrorKind;
use std::net::TcpListener;

const DISCOVERY_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-lookup-binding-v1/golden/discovery-response.json"
));
const OBSERVATION_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-lookup-binding-v1/golden/observation-response.json"
));
fn lookup_request(operation: &str, correlation: &str) -> Result<HttpRequest, String> {
    let mut request =
        authenticated_request("/v1/instances/instance-1/game-information/lookup-binding");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        correlation.to_owned(),
    );
    request.body = serde_json::to_vec(&json!({
        "operation": operation,
        "project_id": "proj-1",
        "run_id": "run-42",
        "episode_id": "episode-7",
        "agent_id": "agent-3",
        "authority_epoch": 7
    }))
    .map_err(|error| error.to_string())?;
    Ok(request)
}

fn service_with_address(address: String) -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    service.config.mod_address = address;
    Ok(service)
}

#[test]
fn lookup_binding_route_forwards_fixed_path_and_validates_discovery_and_observation()
-> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(
        listener,
        vec![
            (200, DISCOVERY_RESPONSE.to_vec()),
            (200, OBSERVATION_RESPONSE.to_vec()),
        ],
    );
    let mut service = service_with_address(address)?;

    for (operation, correlation, response) in [
        ("discovery", "corr-lbr-discovery-1", DISCOVERY_RESPONSE),
        ("observe", "corr-lbr-observation-1", OBSERVATION_RESPONSE),
    ] {
        let request = lookup_request(operation, correlation)?;
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 200, "{operation}");
        assert_eq!(body, response, "{operation}");
    }

    let forwarded = worker
        .join()
        .map_err(|_| String::from("lookup-binding producer panicked"))??;
    assert_eq!(forwarded.len(), 2);
    for (request, correlation) in forwarded
        .iter()
        .zip(["corr-lbr-discovery-1", "corr-lbr-observation-1"])
    {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/v1/game-information/lookup-binding");
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer mod-token")
        );
        assert_eq!(
            request
                .headers
                .get("x-sts2-instance-id")
                .map(String::as_str),
            Some("instance-1")
        );
        assert_eq!(
            request
                .headers
                .get("x-sts2-lease-epoch")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            request.headers.get("x-sts2-locale").map(String::as_str),
            Some("en-US")
        );
        assert_eq!(
            request
                .headers
                .get("x-sts2-correlation-id")
                .map(String::as_str),
            Some(correlation)
        );
    }
    assert_eq!(
        forwarded[0].body,
        lookup_request("discovery", "corr-lbr-discovery-1")?.body
    );
    assert_eq!(
        forwarded[1].body,
        lookup_request("observe", "corr-lbr-observation-1")?.body
    );
    Ok(())
}

#[test]
fn malformed_duplicate_foreign_oversized_and_status_mismatch_never_return_success()
-> Result<(), String> {
    let foreign_instance = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["binding"]["instance_id"] = json!("instance-2");
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let foreign_scope = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["binding"]["scope"]["agent_id"] = json!("agent-foreign");
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let foreign_epoch = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["binding"]["authority_epoch"] = json!(8);
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let forged_binding_id = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["binding"]["binding_id"] = json!("0".repeat(64));
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let invalid_nested_schema = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["binding"]["unexpected"] = json!("rejected");
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let foreign_correlation = {
        let mut value: Value =
            serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
        value["correlation_id"] = json!("corr-foreign");
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let mixed_observation = {
        let mut value: Value =
            serde_json::from_slice(OBSERVATION_RESPONSE).map_err(|error| error.to_string())?;
        value["observation"]["binding_id"] = json!("0".repeat(64));
        serde_json::to_vec(&value).map_err(|error| error.to_string())?
    };
    let discovery_text =
        std::str::from_utf8(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
    let duplicate_member =
        format!(r#"{{"correlation_id":"duplicate",{}"#, &discovery_text[1..]).into_bytes();

    for (label, operation, correlation, status, body) in [
        (
            "malformed",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            b"{".to_vec(),
        ),
        (
            "duplicate",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            duplicate_member,
        ),
        (
            "foreign-instance",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            foreign_instance,
        ),
        (
            "foreign-scope",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            foreign_scope,
        ),
        (
            "foreign-authority-epoch",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            foreign_epoch,
        ),
        (
            "forged-binding-id",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            forged_binding_id,
        ),
        (
            "invalid-nested-schema",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            invalid_nested_schema,
        ),
        (
            "foreign-correlation",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            foreign_correlation,
        ),
        (
            "mixed-observation",
            "observe",
            "corr-lbr-observation-1",
            200,
            mixed_observation,
        ),
        (
            "oversized",
            "discovery",
            "corr-lbr-discovery-1",
            200,
            vec![b' '; RESPONSE_LIMIT_BYTES + 1],
        ),
        (
            "status-mismatch",
            "discovery",
            "corr-lbr-discovery-1",
            202,
            DISCOVERY_RESPONSE.to_vec(),
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let address = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .to_string();
        let worker = serve_http_sequence(listener, vec![(status, body)]);
        let mut service = service_with_address(address)?;
        let request = lookup_request(operation, correlation)?;
        let (returned_status, returned_body) = service.handle_request(&request);
        assert_eq!(returned_status, 502, "{label}");
        let error: Value =
            serde_json::from_slice(&returned_body).map_err(|error| error.to_string())?;
        assert!(
            matches!(
                error["error_code"].as_str(),
                Some(
                    "game_information_lookup_binding_response_invalid"
                        | "game_information_response_oversized"
                )
            ),
            "{label}: {error}"
        );
        worker
            .join()
            .map_err(|_| format!("{label} producer panicked"))??;
    }
    Ok(())
}

#[test]
fn valid_typed_lookup_binding_errors_keep_their_non_success_status() -> Result<(), String> {
    let mut value: Value =
        serde_json::from_slice(DISCOVERY_RESPONSE).map_err(|error| error.to_string())?;
    value["kind"] = json!("error_response");
    value["binding"] = Value::Null;
    value["discovery"] = Value::Null;
    value["observation"] = Value::Null;
    value["error"] = json!({
        "code": "malformed",
        "field": null,
        "reason": null
    });
    let response = serde_json::to_vec(&value).map_err(|error| error.to_string())?;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(listener, vec![(409, response.clone())]);
    let mut service = service_with_address(address)?;
    let request = lookup_request("discovery", "corr-lbr-discovery-1")?;
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 409);
    assert_eq!(body, response);
    worker
        .join()
        .map_err(|_| String::from("typed-error producer panicked"))??;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_http_sequence(listener, vec![(200, response)]);
    let mut service = service_with_address(address)?;
    let request = lookup_request("discovery", "corr-lbr-discovery-1")?;
    assert_eq!(service.handle_request(&request).0, 502);
    worker
        .join()
        .map_err(|_| String::from("successful-status error producer panicked"))??;
    Ok(())
}

#[test]
fn authentication_and_stale_lease_reject_before_lookup_binding_forwarding() -> Result<(), String> {
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut service = service_with_address(address)?;

    let mut unauthorized = lookup_request("discovery", "corr-lbr-discovery-1")?;
    unauthorized
        .headers
        .insert(String::from("authorization"), String::from("Bearer wrong"));
    assert_eq!(service.handle_request(&unauthorized).0, 401);

    let mut stale_lease = lookup_request("discovery", "corr-lbr-discovery-1")?;
    stale_lease
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("2"));
    assert_eq!(service.handle_request(&stale_lease).0, 409);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));
    Ok(())
}
