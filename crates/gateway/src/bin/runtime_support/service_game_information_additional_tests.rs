// SPDX-License-Identifier: MIT

use super::super::game_information_forwarder::GameInformationForwarder;
use super::super::game_information_payload::MAX_MESSAGE_BYTES;
use super::test_support::{authenticated_request, test_service};
use super::*;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

const STATIC_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-request.json"
));
const STATIC_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-response.json"
));
const STATIC_PAGE_TWO_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-2-request.json"
));
const STATIC_PAGE_TWO_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/static-page-2-response.json"
));
const LIVE_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-query-v1/golden/live-detail-request.json"
));

fn query_request(path: &str, body: &[u8]) -> HttpRequest {
    let mut request = authenticated_request(path);
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = body.to_vec();
    if let Ok(value) = serde_json::from_slice::<Value>(body)
        && let Some(correlation) = value.get("correlation_id").and_then(Value::as_str)
    {
        request.headers.insert(
            String::from("x-sts2-correlation-id"),
            correlation.to_owned(),
        );
    }
    request
}

fn replace(body: &[u8], from: &str, to: &str) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    if text.matches(from).count() != 1 {
        return Err(format!("expected one occurrence of {from:?}"));
    }
    Ok(text.replacen(from, to, 1).into_bytes())
}

fn replace_all(body: &[u8], from: &str, to: &str) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    Ok(text.replace(from, to).into_bytes())
}

fn service_with_address(address: String) -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    service.config.mod_address = address;
    Ok(service)
}

fn serve_once(
    listener: TcpListener,
    response: Vec<u8>,
) -> thread::JoinHandle<Result<HttpRequest, String>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let request = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
        write_response(&mut stream, 200, &response).map_err(|error| error.to_string())?;
        Ok(request)
    })
}

#[test]
fn typed_producer_errors_are_preserved_and_oversized_output_is_bounded() -> Result<(), String> {
    let error = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/game-information-query-v1/golden/error-stale-cursor.json"
    ));
    let request_body = replace(STATIC_REQUEST, "corr-static-page-1", "corr-stale-cursor")?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = thread::spawn({
        let error = error.to_vec();
        move || -> Result<(), String> {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let _ = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
            write_response(&mut stream, 409, &error).map_err(|error| error.to_string())
        }
    });
    let mut service = service_with_address(address)?;
    let request = query_request(
        "/v1/instances/instance-1/game-information/list",
        &request_body,
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 409);
    assert_eq!(body, error);
    worker
        .join()
        .map_err(|_| String::from("error producer panicked"))??;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let _ = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
        write_response(&mut stream, 200, &vec![b' '; MAX_MESSAGE_BYTES + 1])
            .map_err(|error| error.to_string())
    });
    service.config.mod_address = address;
    let request = query_request(
        "/v1/instances/instance-1/game-information/list",
        STATIC_REQUEST,
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 502);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "game_information_response_oversized");
    worker
        .join()
        .map_err(|_| String::from("oversized producer panicked"))??;
    Ok(())
}

#[test]
fn cursor_continuations_are_bound_to_the_complete_query_and_release_after_timeout()
-> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = serve_once(listener, STATIC_RESPONSE.to_vec());
    let mut service = service_with_address(address)?;
    let first = query_request(
        "/v1/instances/instance-1/game-information/list",
        STATIC_REQUEST,
    );
    assert_eq!(service.handle_request(&first).0, 200);
    worker
        .join()
        .map_err(|_| String::from("cursor producer panicked"))??;

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let page_two_worker = serve_once(listener, STATIC_PAGE_TWO_RESPONSE.to_vec());
    service.config.mod_address = address;
    let page_two = query_request(
        "/v1/instances/instance-1/game-information/list",
        STATIC_PAGE_TWO_REQUEST,
    );
    let (status, body) = service.handle_request(&page_two);
    assert_eq!(status, 200);
    assert_eq!(body, STATIC_PAGE_TWO_RESPONSE);
    let forwarded = page_two_worker
        .join()
        .map_err(|_| String::from("page-two producer panicked"))??;
    assert_eq!(forwarded.path, "/api/v1/game-information/list");
    assert_eq!(forwarded.body, STATIC_PAGE_TWO_REQUEST);

    let cross_locale = replace(
        STATIC_PAGE_TWO_REQUEST,
        "\"locale\":\"en-US\"",
        "\"locale\":\"fr-FR\"",
    )?;
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service.config.mod_address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request = query_request(
        "/v1/instances/instance-1/game-information/list",
        &cross_locale,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let worker = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let _ = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
        thread::sleep(Duration::from_millis(500));
        Ok(())
    });
    service.config.mod_address = address;
    service.game_information_exchange_timeout = Duration::from_millis(100);
    let started = Instant::now();
    let request = query_request(
        "/v1/instances/instance-1/game-information/list",
        STATIC_REQUEST,
    );
    assert_eq!(service.handle_request(&request).0, 504);
    assert!(started.elapsed() < Duration::from_millis(400));
    worker
        .join()
        .map_err(|_| String::from("timeout producer panicked"))??;
    Ok(())
}

#[test]
fn live_scope_requires_the_configured_run_and_snapshot_identity() -> Result<(), String> {
    let mut service = test_service()?;
    service.config.game_information_run_id = String::from("run-2");
    service.game_information = GameInformationForwarder::new(
        super::super::game_information_forwarder::MAX_REQUEST_BYTES,
        super::super::game_information_forwarder::MAX_RESPONSE_BYTES,
        &service.config.game_information_content_manifest_id,
        &service.config.game_information_run_id,
    );
    let request = query_request(
        "/v1/instances/instance-1/game-information/detail",
        LIVE_REQUEST,
    );
    assert_eq!(service.handle_request(&request).0, 409);
    Ok(())
}

#[test]
fn content_and_instance_authorities_do_not_share_cursor_state() -> Result<(), String> {
    let content_two_request = replace_all(STATIC_REQUEST, "content-1", "content-2")?;
    let content_two_response = replace_all(STATIC_RESPONSE, "content-1", "content-2")?;
    let mut service_one = test_service()?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_once(listener, STATIC_RESPONSE.to_vec());
    service_one.config.mod_address = address;
    let first = query_request(
        "/v1/instances/instance-1/game-information/list",
        STATIC_REQUEST,
    );
    assert_eq!(service_one.handle_request(&first).0, 200);
    producer
        .join()
        .map_err(|_| String::from("first producer panicked"))??;

    let mut service_two = test_service()?;
    service_two.config.instance_id = String::from("instance-2");
    service_two.config.game_information_content_manifest_id = String::from("content-2");
    service_two.game_information = GameInformationForwarder::new(
        super::super::game_information_forwarder::MAX_REQUEST_BYTES,
        super::super::game_information_forwarder::MAX_RESPONSE_BYTES,
        "content-2",
        "run-1",
    );
    let trap = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    service_two.config.mod_address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let mut foreign_cursor = query_request(
        "/v1/instances/instance-2/game-information/list",
        STATIC_PAGE_TWO_REQUEST,
    );
    foreign_cursor.headers.insert(
        String::from("x-sts2-instance-id"),
        String::from("instance-2"),
    );
    assert_eq!(service_two.handle_request(&foreign_cursor).0, 409);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == ErrorKind::WouldBlock
    ));

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let producer = serve_once(listener, content_two_response.clone());
    service_two.config.mod_address = address;
    let mut isolated = query_request(
        "/v1/instances/instance-2/game-information/list",
        &content_two_request,
    );
    isolated.headers.insert(
        String::from("x-sts2-instance-id"),
        String::from("instance-2"),
    );
    let (status, body) = service_two.handle_request(&isolated);
    assert_eq!(status, 200);
    assert_eq!(body, content_two_response);
    let forwarded = producer
        .join()
        .map_err(|_| String::from("second producer panicked"))??;
    assert_eq!(
        forwarded
            .headers
            .get("x-sts2-instance-id")
            .map(String::as_str),
        Some("instance-2")
    );
    assert_eq!(forwarded.body, content_two_request);
    Ok(())
}
