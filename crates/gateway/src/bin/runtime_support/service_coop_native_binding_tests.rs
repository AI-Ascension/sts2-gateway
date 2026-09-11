// SPDX-License-Identifier: MIT

use super::coop_native_tests::{
    LEGAL_CATALOG_REQUEST, LEGAL_CATALOG_RESPONSE, OBSERVATION, RECOVERED_ACTION_REQUEST,
    RECOVERED_ACTION_RESPONSE, UNKNOWN_ACTION_REQUEST, UNKNOWN_ACTION_RESPONSE, configure_for,
    fixture, serve_once,
};
use super::test_support::test_service;
use serde_json::Value;
use std::net::TcpListener;

fn wrong_local_peer_response(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut response = fixture(bytes);
    let Some(peer) = response["observation"]["peers"]
        .as_array_mut()
        .and_then(|peers| {
            peers
                .iter_mut()
                .find(|peer| peer["role"].as_str() == Some("local"))
        })
    else {
        return Err(String::from("fixture must have one local peer"));
    };
    peer["peer_token"] = Value::String(String::from("peer:wrong-local"));
    serde_json::to_vec(&response).map_err(|error| error.to_string())
}

fn configured_listener(service: &mut super::RuntimeService) -> Result<TcpListener, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    Ok(listener)
}

#[test]
fn native_observation_rejects_wrong_local_peer() -> Result<(), String> {
    let observation = fixture(OBSERVATION);
    let mut service = test_service()?;
    let request = configure_for(&mut service, &observation);
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/observation",
        200,
        wrong_local_peer_response(OBSERVATION)?,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;
    Ok(())
}

#[test]
fn native_catalog_rejects_wrong_local_peer_and_response_actor() -> Result<(), String> {
    let catalog = fixture(LEGAL_CATALOG_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &catalog);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/legal-catalog",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), String::from("application/json"));
    request.body = LEGAL_CATALOG_REQUEST.to_vec();
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/legal-catalog",
        200,
        wrong_local_peer_response(LEGAL_CATALOG_RESPONSE)?,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;

    let mut response_actor_mismatch = fixture(LEGAL_CATALOG_RESPONSE);
    response_actor_mismatch["actor_peer"] = Value::String(String::from("peer:wrong-actor"));
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/legal-catalog",
        200,
        serde_json::to_vec(&response_actor_mismatch).map_err(|error| error.to_string())?,
    );
    // The closed response relation rejects an actor drift before the route-ownership check.
    assert_eq!(service.handle_request(&request).0, 502);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;
    Ok(())
}

#[test]
fn native_action_rejects_wrong_local_peer_without_clearing_pending() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!("/v1/instances/{}/coop/native/action", service.config.instance_id);
    request.headers.insert("content-type".into(), String::from("application/json"));
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/action",
        200,
        wrong_local_peer_response(UNKNOWN_ACTION_RESPONSE)?,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;
    assert!(service.coop_native_pending.is_some());
    Ok(())
}

#[test]
fn native_recovery_rejects_wrong_local_peer_without_clearing_pending() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!("/v1/instances/{}/coop/native/action", service.config.instance_id);
    request.headers.insert("content-type".into(), String::from("application/json"));
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(listener, "/api/v1/coop/native/action", 200, UNKNOWN_ACTION_RESPONSE.to_vec());
    assert_eq!(service.handle_request(&request).0, 200);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;

    let recovered = fixture(RECOVERED_ACTION_REQUEST);
    let mut recovery = configure_for(&mut service, &recovered);
    recovery.method = String::from("POST");
    recovery.path = format!("/v1/instances/{}/coop/native/recover", service.config.instance_id);
    recovery.headers.insert("content-type".into(), String::from("application/json"));
    recovery.body = RECOVERED_ACTION_REQUEST.to_vec();
    let listener = configured_listener(&mut service)?;
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/recover",
        200,
        wrong_local_peer_response(RECOVERED_ACTION_RESPONSE)?,
    );
    assert_eq!(service.handle_request(&recovery).0, 409);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;
    assert!(service.coop_native_pending.is_some());
    Ok(())
}

#[test]
fn native_recovery_rejects_authority_change_without_settlement() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = listener.local_addr().map_err(|error| error.to_string())?.to_string();
    let worker = serve_once(listener, "/api/v1/coop/native/action", 200, UNKNOWN_ACTION_RESPONSE.to_vec());
    assert_eq!(service.handle_request(&request).0, 200);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;

    let recovered = fixture(RECOVERED_ACTION_REQUEST);
    let mut recovery = configure_for(&mut service, &recovered);
    recovery.method = String::from("POST");
    recovery.path = format!("/v1/instances/{}/coop/native/recover", service.config.instance_id);
    recovery.headers.insert("content-type".into(), "application/json".into());
    recovery.body = RECOVERED_ACTION_REQUEST.to_vec();
    let mut changed = fixture(RECOVERED_ACTION_RESPONSE);
    for path in ["receipt", "effect"] {
        if let Some(object) = changed.get_mut(path).and_then(Value::as_object_mut) {
            object.insert(String::from("authority_id"), Value::String(String::from("authority:changed-test")));
            object.insert(String::from("authority_epoch"), Value::String(String::from("epoch:changed-test")));
        }
    }
    changed["observation"]["authority_id"] = Value::String(String::from("authority:changed-test"));
    changed["observation"]["host_authority_epoch"] = Value::String(String::from("epoch:changed-test"));
    if let Some(peers) = changed["observation"]["peers"].as_array_mut() {
        for peer in peers {
            peer["authority_id"] = Value::String(String::from("authority:changed-test"));
            peer["authority_epoch"] = Value::String(String::from("epoch:changed-test"));
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener.set_nonblocking(true).map_err(|error| error.to_string())?;
    service.config.mod_address = listener.local_addr().map_err(|error| error.to_string())?.to_string();
    let worker = serve_once(
        listener,
        "/api/v1/coop/native/recover",
        200,
        serde_json::to_vec(&changed).map_err(|error| error.to_string())?,
    );
    // A relation-valid authority change reaches the retained-operation fence, which refuses
    // settlement while preserving reconciliation state.
    assert_eq!(service.handle_request(&recovery).0, 409);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;
    assert!(service.coop_native_pending.is_some());
    Ok(())
}

#[test]
fn native_recovery_rejects_another_operation_or_binding() -> Result<(), String> {
    let action = fixture(UNKNOWN_ACTION_REQUEST);
    let mut service = test_service()?;
    let mut request = configure_for(&mut service, &action);
    request.method = String::from("POST");
    request.path = format!(
        "/v1/instances/{}/coop/native/action",
        service.config.instance_id
    );
    request.headers.insert("content-type".into(), "application/json".into());
    request.body = UNKNOWN_ACTION_REQUEST.to_vec();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener.set_nonblocking(true).map_err(|error| error.to_string())?;
    service.config.mod_address = listener.local_addr().map_err(|error| error.to_string())?.to_string();
    let worker = serve_once(listener, "/api/v1/coop/native/action", 200, UNKNOWN_ACTION_RESPONSE.to_vec());
    assert_eq!(service.handle_request(&request).0, 200);
    worker.join().map_err(|_| String::from("downstream worker panicked"))??;

    let recovered = fixture(RECOVERED_ACTION_REQUEST);
    let mut recovery = configure_for(&mut service, &recovered);
    recovery.method = String::from("POST");
    recovery.path = format!("/v1/instances/{}/coop/native/recover", service.config.instance_id);
    recovery.headers.insert("content-type".into(), "application/json".into());
    let mut wrong_operation = recovered.clone();
    wrong_operation["operation_id"] = Value::String(String::from("op:native:other"));
    recovery.body = serde_json::to_vec(&wrong_operation).map_err(|error| error.to_string())?;
    assert_eq!(service.handle_request(&recovery).0, 409);

    if let Some(binding) = service.coop_native_peer_binding.as_mut() {
        binding.peer_id = String::from("peer:other");
    }
    recovery.body = RECOVERED_ACTION_REQUEST.to_vec();
    assert_eq!(service.handle_request(&recovery).0, 409);
    assert!(service.coop_native_pending.is_some());
    Ok(())
}
