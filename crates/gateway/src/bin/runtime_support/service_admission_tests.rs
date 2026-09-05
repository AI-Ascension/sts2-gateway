// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

#[test]
fn released_or_shutdown_lease_cannot_be_reallocated() -> Result<(), String> {
    for shutdown in [false, true] {
        let mut service = test_service()?;
        let mut request = authenticated_request(if shutdown {
            "/v2/instances/instance-1/shutdown"
        } else {
            "/v1/instances/instance-1/release"
        });
        request.method = String::from("POST");
        let (status, _) = service.handle_request(&request);
        assert_eq!(status, if shutdown { 202 } else { 200 });
        let allocation =
            br#"{"instance_id":"instance-1","caller_id":"harness","session_id":"session-1"}"#;
        let (status, body) = service.allocate(allocation);
        assert_eq!(status, 409);
        let response: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        assert_eq!(response["error_code"], "lease_context_revoked");
        assert!(!service.lease_active);
        let stale = authenticated_request("/v2/instances/instance-1/state");
        assert_eq!(service.handle_request(&stale).0, 409);
    }
    Ok(())
}

#[test]
fn operation_capacity_is_explicitly_bounded() {
    assert_eq!(super::configuration::parse_operation_capacity("1"), Ok(1));
    assert_eq!(
        super::configuration::parse_operation_capacity("64"),
        Ok(super::MAX_OPERATION_CAPACITY)
    );
    assert!(super::configuration::parse_operation_capacity("0").is_err());
    assert!(super::configuration::parse_operation_capacity("65").is_err());
    assert!(super::configuration::parse_operation_capacity("not-a-number").is_err());
}

#[test]
fn queue_capacity_is_explicitly_bounded() {
    assert_eq!(super::configuration::parse_queue_capacity("1"), Ok(1));
    assert_eq!(
        super::configuration::parse_queue_capacity("64"),
        Ok(super::MAX_QUEUE_CAPACITY)
    );
    assert!(super::configuration::parse_queue_capacity("0").is_err());
    assert!(super::configuration::parse_queue_capacity("65").is_err());
    assert!(super::configuration::parse_queue_capacity("not-a-number").is_err());
}

#[test]
fn runtime_endpoints_are_numeric_loopback_addresses() {
    for address in ["127.0.0.1:15525", "127.0.0.2:15526", "[::1]:15525"] {
        assert!(super::configuration::validate_loopback_address("endpoint", address).is_ok());
    }
    for address in [
        "0.0.0.0:15525",
        "[::]:15525",
        "192.0.2.1:15525",
        "localhost:15525",
        "example.com:80",
        "127.0.0.1",
        "127.0.0.1:99999",
        "127.0.0.1:0",
        "[::1]:0",
    ] {
        assert!(super::configuration::validate_loopback_address("endpoint", address).is_err());
    }
}

#[test]
fn operation_overload_is_typed_and_retryable() -> Result<(), String> {
    let (status, body) =
        super::v2::runtime_v2_error(sts2_gateway::RuntimeV2LedgerError::CapacityExceeded);
    assert_eq!(status, 429);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "runtime_v2_operation_capacity");
    assert_eq!(value["retryable"], true);
    assert_eq!(value["retry_after_ms"], 1000);
    Ok(())
}

#[test]
fn metrics_route_is_authenticated_and_reports_queue_capacity() -> Result<(), String> {
    let mut service = test_service()?;
    let request = authenticated_request("/v2/instances/instance-1/metrics");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 200);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["instance_id"], "instance-1");
    assert_eq!(value["queue_capacity"], 8);
    assert_eq!(value["queue_depth"], 0);
    Ok(())
}

#[test]
fn shutdown_route_closes_the_lease_and_marks_admission() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v2/instances/instance-1/shutdown");
    request.method = String::from("POST");
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 202);
    let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["status"], "shutdown_requested");
    assert!(service.shutdown_requested);
    assert!(!service.lease_active);
    Ok(())
}

#[test]
fn shutdown_drains_requests_until_admission_producer_exits() -> Result<(), String> {
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    let service = test_service()?;
    let metrics = service.metrics.clone();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = super::run_worker(service, receiver, Arc::new(AtomicBool::new(true)), address);
        let _ = finished_sender.send(result);
    });
    let mut shutdown_client = TcpStream::connect(address).map_err(|e| e.to_string())?;
    shutdown_client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let (shutdown_stream, _) = listener.accept().map_err(|e| e.to_string())?;
    let mut request = authenticated_request("/v2/instances/instance-1/shutdown");
    request.method = String::from("POST");
    metrics.queue_admitted();
    sender
        .send(super::QueuedRequest {
            stream: shutdown_stream,
            request,
        })
        .map_err(|e| e.to_string())?;
    assert_eq!(
        super::read_response(
            &mut shutdown_client,
            super::Instant::now() + Duration::from_secs(2)
        )
        .map_err(|e| format!("{e:?}"))?
        .status,
        202
    );
    assert!(matches!(
        finished_receiver.recv_timeout(Duration::from_millis(30)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    // Admission was already reading this connection when shutdown began.
    let late_listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let mut late_client =
        TcpStream::connect(late_listener.local_addr().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    late_client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let (late_stream, _) = late_listener.accept().map_err(|e| e.to_string())?;
    metrics.queue_admitted();
    sender
        .send(super::QueuedRequest {
            stream: late_stream,
            request: authenticated_request("/v2/instances/instance-1/metrics"),
        })
        .map_err(|e| e.to_string())?;
    drop(sender);
    assert_eq!(
        super::read_response(
            &mut late_client,
            super::Instant::now() + Duration::from_secs(2)
        )
        .map_err(|e| format!("{e:?}"))?
        .status,
        503
    );
    finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())??;
    worker.join().map_err(|_| String::from("worker panicked"))?;
    assert_eq!(
        metrics.snapshot("instance-1", 2)["cancelled_on_shutdown"],
        1
    );
    assert_eq!(metrics.snapshot("instance-1", 2)["queue_depth"], 0);
    Ok(())
}

#[test]
fn mcp_session_configuration_has_its_own_default_and_validates_overrides() -> Result<(), String> {
    use super::configuration::configured_mcp_session;
    assert_eq!(
        configured_mcp_session(Err(std::env::VarError::NotPresent))?,
        "mcp-session-1"
    );
    assert_eq!(
        configured_mcp_session(Ok("mcp-explicit".to_owned()))?,
        "mcp-explicit"
    );
    for value in [String::new(), "invalid session".to_owned(), "x".repeat(129)] {
        assert!(configured_mcp_session(Ok(value)).is_err());
    }
    assert!(
        configured_mcp_session(Err(std::env::VarError::NotUnicode(
            std::ffi::OsString::from("non-unicode-error-fixture")
        )))
        .is_err()
    );
    Ok(())
}
