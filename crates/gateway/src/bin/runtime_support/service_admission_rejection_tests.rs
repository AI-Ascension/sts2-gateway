// SPDX-License-Identifier: MIT

use super::test_support::*;
use super::*;

#[test]
fn the_refusal_branch_still_accounts_and_answers_exactly_as_before() -> Result<(), String> {
    // The added diagnostic sits inside the `request_rejection` branch of
    // `accept_requests`, so drive that branch over a real socket and pin the
    // behaviour it must not disturb: the two rejection paths keep their own
    // responses, and both keep counting `malformed_rejected`. The
    // served-process evidence for the emitted line is in
    // `crates/gateway/tests/rejected_header_diagnostic.rs`.
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, mpsc};

    const CANARY: &str = "Bearer sts2-114-admission-canary-7b2e5d";

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let admission_open = Arc::new(AtomicBool::new(true));
    let service = test_service()?;
    let metrics = service.metrics.clone();
    let acceptor_metrics = metrics.clone();
    let acceptor_admission = Arc::clone(&admission_open);
    let (sender, receiver) = mpsc::sync_channel::<super::QueuedRequest>(1);
    // A real worker, so the admitted request below is served rather than left
    // queued with nobody to answer it. Only the admitted request reaches it;
    // the other two are refused in admission.
    let worker_admission = Arc::clone(&admission_open);
    let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = super::run_worker(service, receiver, worker_admission, address);
        let _ = finished_sender.send(result);
    });
    let acceptor_sender = sender.clone();
    let acceptor = std::thread::spawn(move || {
        super::accept_requests(
            listener,
            acceptor_sender,
            acceptor_admission,
            AuthPolicy::test_all("gateway-token"),
            String::from("instance-1"),
            false,
            acceptor_metrics,
        )
    });

    let exchange = |raw: String| -> Result<(u16, String), String> {
        let mut client =
            TcpStream::connect(address).map_err(|error| error.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        client
            .write_all(raw.as_bytes())
            .map_err(|error| error.to_string())?;
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .map_err(|error| error.to_string())?;
        let text = String::from_utf8_lossy(&response).into_owned();
        let status = text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or_else(|| format!("no status line in {text}"))?;
        Ok((status, text))
    };

    // The blank line that ends the header block must come last, so any extra
    // header is appended before it rather than parsed as a second request.
    let base = String::from("Host: 127.0.0.1\r\nConnection: close\r\n");
    let end = String::from("\r\n");
    // A refused header: 400 naming the header, counted as malformed, exactly
    // as before this change.
    let (status, body) = exchange(format!(
        "GET /health/live HTTP/1.1\r\n{base}x-unlisted-credential: {CANARY}\r\n{end}"
    ))?;
    assert_eq!(status, 400);
    assert!(body.contains("unsupported_header"), "{body}");
    assert!(body.contains("x-unlisted-credential"), "{body}");
    assert!(!body.contains(CANARY), "a value must not be echoed: {body}");
    assert_eq!(metrics.snapshot("instance-1", 1)["malformed_rejected"], 1);

    // A malformed request: its own response and its own increment, unchanged.
    let (status, body) = exchange(String::from("not-an-http-request\r\n\r\n"))?;
    assert_eq!(status, 400);
    assert!(body.contains("malformed_request"), "{body}");
    assert_eq!(metrics.snapshot("instance-1", 1)["malformed_rejected"], 2);

    // An admitted request: still authenticated, so the branch is not taken and
    // the counter does not move.
    let (status, body) = exchange(format!(
        "GET /health/live HTTP/1.1\r\n{base}authorization: Bearer gateway-token\r\n{end}"
    ))?;
    assert_eq!(status, 200, "{body}");
    assert_eq!(metrics.snapshot("instance-1", 1)["malformed_rejected"], 2);

    admission_open.store(false, Ordering::Release);
    super::admission::wake_listener(address);
    acceptor
        .join()
        .map_err(|_| String::from("acceptor panicked"))??;
    drop(sender);
    finished_receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| error.to_string())??;
    worker
        .join()
        .map_err(|_| String::from("worker panicked"))?;
    Ok(())
}
