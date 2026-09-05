// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

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

#[test]
fn v1_fixed_forwarding_preserves_paths_credentials_and_identity() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(3);
    let worker = thread::spawn(move || -> Result<(), String> {
        for _ in 0..3 {
            let expires = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < expires =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            let request = read_request(&mut stream).map_err(|e| e.to_string())?;
            sender.send(request).map_err(|e| e.to_string())?;
            write_response(&mut stream, 200, br#"{"synthetic":true}"#)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    });
    let mut service = test_service()?;
    service.config.mod_address = address.to_string();
    for (method, public_path, downstream_path, body) in [
        ("GET", "/health/ready", "/health/ready", b"".as_slice()),
        (
            "GET",
            "/v1/instances/instance-1/state",
            "/api/v1/runtime/state",
            b"",
        ),
        (
            "POST",
            "/v1/instances/instance-1/action",
            "/api/v1/runtime/action",
            br#"{"action":"synthetic"}"#,
        ),
    ] {
        let mut request = authenticated_request(public_path);
        request.method = method.to_owned();
        request.body = body.to_vec();
        if !body.is_empty() {
            request
                .headers
                .insert("content-type".to_owned(), "application/json".to_owned());
        }
        let (status, response) = service.handle_request(&request);
        assert_eq!(status, 200);
        if public_path != "/health/ready" {
            assert_eq!(response, br#"{"synthetic":true}"#);
        }
        let forwarded = receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        assert_eq!(forwarded.method, method);
        assert_eq!(forwarded.path, downstream_path);
        assert_eq!(forwarded.body, body);
        assert_eq!(
            forwarded.headers.get("authorization").map(String::as_str),
            Some("Bearer mod-token")
        );
        assert!(!forwarded.headers.contains_key("x-mcp-session-id"));
        if public_path == "/health/ready" {
            assert!(!forwarded.headers.contains_key("x-sts2-correlation-id"));
        } else {
            for (name, expected) in [
                ("x-sts2-instance-id", "instance-1"),
                ("x-sts2-caller-id", "harness"),
                ("x-sts2-session-id", "session-1"),
                ("x-sts2-lease-id", "lease-1"),
                ("x-sts2-lease-epoch", "1"),
                ("x-sts2-correlation-id", "corr-state"),
            ] {
                assert_eq!(
                    forwarded.headers.get(name).map(String::as_str),
                    Some(expected)
                );
            }
        }
    }
    worker
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    Ok(())
}
