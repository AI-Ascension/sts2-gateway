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
            cancellation: super::RequestCancellation::new(),
            cancellation_watcher: None,
        })
        .map_err(|e| e.to_string())?;
    assert_eq!(
        super::super::http::read_response(
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
            cancellation: super::RequestCancellation::new(),
            cancellation_watcher: None,
        })
        .map_err(|e| e.to_string())?;
    drop(sender);
    assert_eq!(
        super::super::http::read_response(
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
fn game_information_http_disconnect_cancels_the_producer_read() -> Result<(), String> {
    use std::io::Read;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const CAPABILITIES_RESPONSE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/game-information-query-v1/golden/capabilities-response.json"
    ));
    const QUERY_REQUEST: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/game-information-query-v1/golden/static-page-1-request.json"
    ));

    fn send_request(
        stream: &mut TcpStream,
        request: &HttpRequest,
        expires: Instant,
    ) -> Result<(), String> {
        let mut headers = request.headers.clone();
        headers.insert(String::from("Content-Length"), request.body.len().to_string());
        super::super::http::write_request(
            stream,
            &request.method,
            &request.path,
            &headers,
            &request.body,
            expires,
        )
        .map_err(|error| error.to_string())
    }

    let mod_listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let mod_address = mod_listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let (producer_started_sender, producer_started_receiver) = mpsc::sync_channel(1);
    let producer = std::thread::spawn(move || -> Result<(), String> {
        let (mut capabilities_stream, _) = mod_listener
            .accept()
            .map_err(|error| error.to_string())?;
        let _ = super::super::http::read_request(&mut capabilities_stream)
            .map_err(|error| format!("{error:?}"))?;
        super::super::http::write_response(&mut capabilities_stream, 200, CAPABILITIES_RESPONSE)
            .map_err(|error| error.to_string())?;

        let (mut query_stream, _) = mod_listener.accept().map_err(|error| error.to_string())?;
        let query = super::super::http::read_request(&mut query_stream)
            .map_err(|error| format!("{error:?}"))?;
        producer_started_sender
            .send(query.path)
            .map_err(|error| error.to_string())?;
        query_stream
            .set_read_timeout(Some(Duration::from_millis(25)))
            .map_err(|error| error.to_string())?;
        let mut byte = [0_u8; 1];
        loop {
            match query_stream.read(&mut byte) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    });

    let gateway_listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let gateway_address = gateway_listener
        .local_addr()
        .map_err(|error| error.to_string())?;
    let mut service = test_service()?;
    service.config.mod_address = mod_address;
    service.game_information_exchange_timeout = Duration::from_millis(800);
    let admission_open = Arc::new(AtomicBool::new(true));
    let (sender, receiver) = std::sync::mpsc::sync_channel(2);
    let (finished_sender, finished_receiver) = std::sync::mpsc::sync_channel(1);
    let worker_admission = Arc::clone(&admission_open);
    let worker = std::thread::spawn(move || {
        let result = super::run_worker(service, receiver, worker_admission, gateway_address);
        let _ = finished_sender.send(result);
    });

    let accept_admission = Arc::clone(&admission_open);
    let accept_auth = {
        // `accept_requests` owns the policy value while the worker owns the
        // service, so clone the immutable authentication/configuration fences.
        AuthPolicy::test_all("gateway-token")
    };
    let accept_metrics = RuntimeMetrics::default();
    let accept_instance = String::from("instance-1");
    let accept_listener = std::thread::spawn(move || {
        super::accept_requests(
            gateway_listener,
            sender,
            accept_admission,
            accept_auth,
            accept_instance,
            false,
            accept_metrics,
        )
    });

    let mut capabilities_client =
        TcpStream::connect(gateway_address).map_err(|error| error.to_string())?;
    let capabilities_request = super::test_support::game_information_capabilities_request();
    send_request(
        &mut capabilities_client,
        &capabilities_request,
        Instant::now() + Duration::from_secs(2),
    )?;
    let capabilities_response = super::super::http::read_response(
        &mut capabilities_client,
        Instant::now() + Duration::from_secs(2),
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(capabilities_response.status, 200);
    let _ = capabilities_client.shutdown(Shutdown::Both);

    let mut query_client = TcpStream::connect(gateway_address).map_err(|error| error.to_string())?;
    let query_request = {
        let mut request = super::test_support::authenticated_request(
            "/v1/instances/instance-1/game-information/query",
        );
        request.method = String::from("POST");
        request.headers.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
        request.headers.insert(
            String::from("x-sts2-correlation-id"),
            String::from("corr-static-page-1"),
        );
        request.body = QUERY_REQUEST.to_vec();
        request
    };
    send_request(
        &mut query_client,
        &query_request,
        Instant::now() + Duration::from_secs(2),
    )?;
    let producer_path = producer_started_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(producer_path, "/api/v1/game-information/list");
    let started = Instant::now();
    let _ = query_client.shutdown(Shutdown::Both);
    producer
        .join()
        .map_err(|_| String::from("producer panicked"))??;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "producer stayed occupied after caller disconnect"
    );

    admission_open.store(false, Ordering::Release);
    super::admission::wake_listener(gateway_address);
    accept_listener
        .join()
        .map_err(|_| String::from("acceptor panicked"))??;
    finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())??;
    worker
        .join()
        .map_err(|_| String::from("worker panicked"))?;
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
