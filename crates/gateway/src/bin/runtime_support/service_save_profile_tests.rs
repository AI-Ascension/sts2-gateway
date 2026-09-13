// SPDX-License-Identifier: MIT

use super::*;
use std::io::{Read, Write};
use super::test_support::{authenticated_request, test_service};

fn response_body(operation_id: &str) -> Vec<u8> {
    format!(
        r#"{{"status":"settled","instance_id":"instance-1","caller_id":"harness","session_id":"session-1","lease_id":"lease-1","lease_epoch":1,"correlation_id":"corr-state","operation_id":"{operation_id}","profiles":[]}}"#
    )
    .into_bytes()
}

fn serve_list(listener: TcpListener, operation_id: &str) -> thread::JoinHandle<Result<String, String>> {
    let response = response_body(operation_id);
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err(String::from("downstream closed before request"));
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream
            .write_all(header.as_bytes())
            .and_then(|_| stream.write_all(&response))
            .map_err(|error| error.to_string())?;
        String::from_utf8(request).map_err(|error| error.to_string())
    })
}

#[test]
fn list_route_forwards_only_the_fixed_downstream_target() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = serve_list(listener, "op-list");
    let mut service = test_service()?;
    service.save_profile.forwarding_mut().clone_from(
        &super::super::save_profile_forwarder::HttpSaveProfileForwarder::new(
            &address.to_string(),
            "mod-token",
        ),
    );
    let mut request = authenticated_request("/v1/instances/instance-1/save-profiles");
    request
        .headers
        .insert(String::from("x-mcp-request-id"), String::from("op-list"));
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    let response: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(response["status"], "settled");
    let wire = server
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    assert!(wire.starts_with("GET /api/v1/save-profiles HTTP/1.1\r\n"));
    assert!(wire.contains("authorization: Bearer mod-token\r\n"));
    assert!(wire.contains("x-sts2-instance-id: instance-1\r\n"));
    assert!(!wire.contains("op-list"));
    Ok(())
}

#[test]
fn unknown_route_and_stale_epoch_are_rejected_before_forwarding() -> Result<(), String> {
    let mut service = test_service()?;
    let unknown = authenticated_request("/v1/instances/instance-1/save-profile/reset");
    assert_eq!(service.handle_request(&unknown).0, 404);

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    service.save_profile.forwarding_mut().clone_from(
        &super::super::save_profile_forwarder::HttpSaveProfileForwarder::new(
            &address.to_string(),
            "mod-token",
        ),
    );
    let mut stale = authenticated_request("/v1/instances/instance-1/save-profiles");
    stale
        .headers
        .insert(String::from("x-mcp-request-id"), String::from("op-stale"));
    stale
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("2"));
    assert_eq!(service.handle_request(&stale).0, 409);
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn mutation_bodies_are_closed_and_active_runs_are_fenced() -> Result<(), String> {
    let mut service = test_service()?;
    let mut select =
        authenticated_request("/v1/instances/instance-1/save-profile/select");
    select.method = String::from("POST");
    select.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    select.headers.insert(
        String::from("x-mcp-request-id"),
        String::from("op-select"),
    );
    select.body = br#"{"profile_id":"slot-1","baseline":{"identity":"base","digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"extra":true}"#.to_vec();
    let (status, body) = service.handle_request(&select);
    assert_eq!(status, 400);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "save_profile_body_fields_invalid"
    );

    service.save_profile_active_run = true;
    let mut create =
        authenticated_request("/v1/instances/instance-1/save-profile/create-disposable");
    create.method = String::from("POST");
    create.headers.insert(
        String::from("x-mcp-request-id"),
        String::from("op-create"),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 409);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "save_profile_active_run"
    );
    Ok(())
}
