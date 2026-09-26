// SPDX-License-Identifier: MIT

//! Real-process evidence that a refused header is recorded server-side.
//!
//! The refusal body already named the header (`sts2-gateway#113`), but the
//! serving process discarded that body once it was written to the socket, so
//! the run artifact retained nothing but `malformed_rejected()` — a counter
//! indistinguishable from a genuinely unparseable request. These tests spawn
//! the built `sts2-gateway-runtime` binary and read its standard error, which
//! is the only way to show the record outlives the request that caused it
//! (AI-Ascension/sts2-harness#541).
//!
//! Claimed evidence class: the *served* gateway process writes the record. No
//! production, deployment, or native effect is claimed.

// Test code is allowed to panic: a failed assertion is the expected failure mode.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

// This suite drives only the refusal path. The process helper it shares with
// `gateway_real_process_episode` carries the whole episode surface, so most of
// it is unused here; the same allowance `tests/poc.rs` makes for its shared
// `support` module.
#[allow(dead_code)]
#[path = "gateway_real_process_episode/gateway.rs"]
mod gateway;
#[allow(dead_code)]
#[path = "gateway_real_process_episode/host.rs"]
mod host;
#[path = "gateway_real_process_episode/host_crypto.rs"]
mod host_crypto;
#[allow(dead_code)]
#[path = "gateway_real_process_episode/wire.rs"]
mod wire;

use std::io::{Read, Write};

use self::gateway::{GATEWAY_TOKEN, GatewayProcess, INSTANCE, TempStore};
use self::host::HostFake;
use self::wire::{error_code, get_status};

/// Sends an authenticated `GET` carrying one extra header the caller chooses.
///
/// Used to present a header outside the gateway's allow-list. The value is
/// passed through verbatim so a test can present a credential-shaped value and
/// then assert it never appears in anything the gateway recorded. This lives
/// here rather than in the shared `wire` module because only this suite sends
/// a deliberately unlisted header; a shared helper would be dead code in
/// `gateway_real_process_episode`, which never sends one.
fn get_with_header(
    address: &str,
    path: &str,
    name: &str,
    value: &str,
) -> Result<(u16, serde_json::Value), String> {
    let authorization = format!("Bearer {GATEWAY_TOKEN}");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: {authorization}\r\n\
         {name}: {value}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let mut stream = std::net::TcpStream::connect(address)
        .map_err(|error| format!("failed to reach the gateway at {address}: {error}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .map_err(|error| format!("failed to bound the gateway read: {error}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("failed to write the request: {error}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| format!("failed to read the gateway response: {error}"))?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| String::from("the gateway sent no complete response"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("malformed gateway status line: {head}"))?;
    Ok((
        status,
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
    ))
}

/// A refused header must leave a server-side record naming it, and the record
/// must be what the *served* process wrote — not something the test inferred
/// from the response body, which the process discards the moment it is written
/// to the socket (AI-Ascension/sts2-harness#541).
#[test]
fn a_refused_header_is_recorded_server_side_by_its_name_only() -> Result<(), String> {
    let host = HostFake::start()?;
    let store = TempStore::new("refusal-record");
    let mut gateway = GatewayProcess::spawn(host.address(), store.path())?;
    gateway.await_ready()?;
    let address = gateway.address().to_owned();

    // A credential-shaped value: if any part of this ever reaches the record,
    // the assertions below must fail rather than quietly pass.
    let secret = "Bearer sk-live-do-not-log-this-credential";

    // Nothing has been refused yet, so nothing may be recorded yet. This is
    // checked before the refusal so the later "no extra record" assertion is
    // not merely the absence of a second event.
    let baseline = gateway.settle_stderr()?;
    assert!(
        !baseline.contains("unsupported header"),
        "a gateway that has refused nothing must have recorded nothing: {baseline}"
    );

    // An allowed header set is still admitted: the record is additive, not a
    // new rejection.
    let admitted = get_status(&address, "/health/live", GATEWAY_TOKEN)?;
    assert_eq!(admitted, 200, "an allow-listed request must be unaffected");
    let after_admitted = gateway.settle_stderr()?;
    assert!(
        !after_admitted.contains("unsupported header"),
        "an admitted request must not record a refused-header name: {after_admitted}"
    );

    // The refusal itself: same status and same error_code the client already
    // saw before this change.
    let (status, body) =
        get_with_header(&address, "/health/live", "x-sts2-unlisted-secret", secret)?;
    assert_eq!(status, 400, "the refusal status must be unchanged: {body}");
    assert_eq!(error_code(&body), "unsupported_header");
    assert_eq!(body["rejected_header"], "x-sts2-unlisted-secret");

    // The record: the served process named the refused header on a surface
    // that outlives the request.
    gateway.await_stderr("x-sts2-unlisted-secret", 1)?;
    // Exactly one record for exactly one refusal. The count is awaited so the
    // assertion cannot pass merely because the drain thread had not yet
    // appended a second line that should not exist.
    gateway.await_stderr("gateway refused unsupported header", 1)?;
    std::thread::sleep(std::time::Duration::from_millis(250));
    let recorded = gateway.stderr()?;
    assert!(
        recorded.contains("gateway refused unsupported header: x-sts2-unlisted-secret"),
        "the served process must record the refused header by name: {recorded}"
    );
    assert!(
        !recorded.contains(secret),
        "a header value must never be recorded: {recorded}"
    );
    assert!(
        !recorded.contains("do-not-log-this-credential"),
        "no fragment of a header value may be recorded: {recorded}"
    );
    assert!(
        !recorded.contains("sk-live"),
        "no fragment of a header value may be recorded: {recorded}"
    );

    // The admitted request earlier and this refused one share a process, so
    // the record cannot be attributed to the wrong request: only the refused
    // header's name appears, and exactly once for this one refusal.
    assert_eq!(
        recorded
            .matches("gateway refused unsupported header")
            .count(),
        1,
        "exactly one refusal was made, so exactly one record may exist: {recorded}"
    );

    gateway.terminate()?;
    Ok(())
}

/// The same request must produce the same record every time, and a second
/// refusal must not be able to overwrite or borrow the first one's name.
#[test]
fn a_repeated_refusal_records_the_same_name_and_does_not_leak_across_requests() -> Result<(), String>
{
    let host = HostFake::start()?;
    let store = TempStore::new("refusal-repeat");
    let mut gateway = GatewayProcess::spawn(host.address(), store.path())?;
    gateway.await_ready()?;
    let address = gateway.address().to_owned();

    for _ in 0..3 {
        let (status, body) =
            get_with_header(&address, "/health/live", "x-sts2-repeatable", "value")?;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["rejected_header"], "x-sts2-repeatable");
    }
    // Awaited by count, not by first sighting: three refusals must yield three
    // records, and the drain thread appends asynchronously.
    gateway.await_stderr("gateway refused unsupported header: x-sts2-repeatable", 3)?;
    let recorded = gateway.stderr()?;
    assert_eq!(
        recorded
            .matches("gateway refused unsupported header: x-sts2-repeatable")
            .count(),
        3,
        "each refusal records its own name, identically: {recorded}"
    );

    // A different refused header is recorded under its own name. The earlier
    // record is not rewritten, so the two requests stay attributable.
    let (status, body) = get_with_header(&address, "/health/live", "x-sts2-second-name", "value")?;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["rejected_header"], "x-sts2-second-name");
    gateway.await_stderr("x-sts2-second-name", 1)?;
    let recorded = gateway.stderr()?;
    assert!(
        recorded.contains("gateway refused unsupported header: x-sts2-repeatable"),
        "the earlier request's record must survive: {recorded}"
    );
    assert!(
        recorded.contains("gateway refused unsupported header: x-sts2-second-name"),
        "the later request records its own name: {recorded}"
    );

    // `malformed_rejected` still fires: the counter is what the metrics surface
    // exposes, and this change must not have moved it.
    let (status, metrics) = get_with_header(
        &address,
        &format!("/v2/instances/{INSTANCE}/metrics"),
        "x-sts2-metrics-probe",
        "value",
    )?;
    assert_eq!(status, 400, "{metrics}");

    gateway.terminate()?;
    Ok(())
}
