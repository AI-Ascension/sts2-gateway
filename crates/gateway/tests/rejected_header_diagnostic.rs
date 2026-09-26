// SPDX-License-Identifier: MIT

//! Real-process evidence that a refused header is visible in the gateway's own
//! diagnostic output, and that a header *value* never is.
//!
//! The in-process unit tests under `runtime_support/` cover the decision, but
//! this suite spawns the built `sts2-gateway-runtime` binary with
//! `std::process::Command::new` and reads the child's **stderr**, so the line
//! under test is the one an operator actually gets from the served process.
//!
//! The claimed evidence class is exactly that: the served gateway process emits
//! the diagnostic. This is not a production-mode claim, and no native game,
//! deployment, or soak is claimed.

// Test code is allowed to panic: a failed assertion is the expected failure mode.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const TOKEN: &str = "diagnostic-gateway-token";
const INSTANCE: &str = "instance-1";

/// Distinctive enough that a substring collision would be a surprise rather
/// than a coincidence, and carrying a separable fragment so the test can assert
/// that *no* fragment of the value survives, not merely the whole string.
const VALUE_CANARY: &str = "Bearer sts2-114-header-value-canary-9f3a1c";
const CANARY_FRAGMENT: &str = "sts2-114-header-value-canary";

/// The header name the refusal is expected to report. Off the allow-list, so
/// `first_rejected_header` selects it deterministically.
const REFUSED_NAME: &str = "x-unlisted-credential";

const DIAGNOSTIC_LIMIT: usize = 1 << 20;
const READY_DEADLINE: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const IO_DEADLINE: Duration = Duration::from_secs(10);

type Headers = Vec<(String, String)>;

/// A served gateway process whose stderr is drained into a shared buffer.
///
/// The child is killed on drop as a backstop so a failing assertion cannot
/// leave an orphan holding a loopback port.
struct GatewayProcess {
    child: Child,
    address: String,
    diagnostics: Arc<Mutex<String>>,
}

impl GatewayProcess {
    /// Spawns `sts2-gateway-runtime` with no ambient `STS2_*` configuration, so
    /// the served process cannot satisfy an assertion from the test's own
    /// environment.
    fn spawn() -> Result<Self, String> {
        let port = free_loopback_port()?;
        let address = format!("127.0.0.1:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_sts2-gateway-runtime"))
            .env_clear()
            .env("STS2_GATEWAY_ADDR", &address)
            .env("STS2_MOD_ADDR", "127.0.0.1:1")
            .env("STS2_GATEWAY_TOKEN", TOKEN)
            // Required by `RuntimeConfig::from_environment` even though no
            // route under test reaches a mod. The lifecycle is left
            // unconfigured, so nothing is ever forwarded to this endpoint.
            .env("STS2_MOD_TOKEN", "diagnostic-mod-token")
            .env("STS2_INSTANCE_ID", INSTANCE)
            .env("STS2_MCP_SESSION_ID", "mcp-session-1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to spawn sts2-gateway-runtime: {error}"))?;
        let diagnostics = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            drain(stderr, Arc::clone(&diagnostics));
        }
        Ok(Self {
            child,
            address,
            diagnostics,
        })
    }

    fn address(&self) -> &str {
        &self.address
    }

    /// Waits until the served process answers an authenticated liveness probe.
    ///
    /// The readiness route itself stays `503` until a boot is fenced, so the
    /// probe is `/health/live`: it proves the listener, the request parser, and
    /// the auth policy are serving.
    fn await_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "gateway process exited before it served a probe ({status}): {}",
                    self.diagnostics()
                ));
            }
            if call(self.address(), &allowed_headers()) == Ok(200) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "gateway process never answered a liveness probe: {}",
                    self.diagnostics()
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn diagnostics(&self) -> String {
        self.diagnostics
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }

    /// Polls the child's stderr until `predicate` holds or the deadline passes.
    ///
    /// Returns the snapshot either way, so a caller that is asserting on the
    /// *absence* of a line can first wait out the quiet window.
    fn await_diagnostics(&self, predicate: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + IO_DEADLINE;
        loop {
            let snapshot = self.diagnostics();
            if predicate(&snapshot) || Instant::now() >= deadline {
                return snapshot;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for GatewayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sends one request over a fresh loopback connection and returns its status.
fn call(address: &str, headers: &Headers) -> Result<u16, String> {
    let mut stream =
        TcpStream::connect(address).map_err(|error| format!("connect failed: {error}"))?;
    stream
        .set_read_timeout(Some(IO_DEADLINE))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .map_err(|error| error.to_string())?;
    let mut head = format!("GET /health/live HTTP/1.1\r\nHost: {address}\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(head.as_bytes())
        .map_err(|error| format!("request write failed: {error}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|error| format!("response read failed: {error}"))?;
    let text = String::from_utf8_lossy(&raw);
    text.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("no status line in response: {text}"))
}

/// An allowed, authenticated header set.
fn allowed_headers() -> Headers {
    vec![
        (String::from("authorization"), format!("Bearer {TOKEN}")),
        (String::from("x-sts2-instance-id"), String::from(INSTANCE)),
        (
            String::from("x-mcp-session-id"),
            String::from("mcp-session-1"),
        ),
    ]
}

/// The allowed set plus one off-allow-list header whose value is the canary.
fn refused_headers() -> Headers {
    let mut headers = allowed_headers();
    headers.push((String::from(REFUSED_NAME), String::from(VALUE_CANARY)));
    headers
}

/// The count of transcript lines naming the refused header.
fn naming_lines(transcript: &str) -> usize {
    transcript
        .lines()
        .filter(|line| line.contains(REFUSED_NAME))
        .count()
}

/// A refusal must be traceable in the gateway's own output, and must not
/// disclose the value that was refused.
///
/// Non-vacuous in four ways. The header is genuinely off the allow-list, so
/// the refusal is a real branch: the status must be `400` and the response
/// body must independently name the header. The transcript is asserted to name
/// that header exactly once, so an unconditional per-request log fails the
/// count. The canary value is asserted absent from the whole child
/// transcript, and so is a distinctive fragment of it, so a log that echoed any
/// part of the value fails.
#[test]
fn a_refused_header_is_named_in_the_served_process_diagnostic() -> Result<(), String> {
    let mut gateway = GatewayProcess::spawn()?;
    gateway.await_ready()?;

    assert_eq!(call(gateway.address(), &allowed_headers())?, 200);
    let baseline = gateway.await_diagnostics(|_| false);
    assert!(
        !baseline.contains("unsupported"),
        "an admitted request must not be refused: {baseline}"
    );

    assert_eq!(call(gateway.address(), &refused_headers())?, 400);
    let transcript = gateway.await_diagnostics(|text| naming_lines(text) > 0);
    assert_eq!(
        naming_lines(&transcript),
        1,
        "exactly one diagnostic line must name the header: {transcript}"
    );
    let line = transcript
        .lines()
        .find(|line| line.contains(REFUSED_NAME))
        .unwrap_or_default();
    assert!(
        line.contains("unsupported"),
        "the diagnostic must identify the refusal: {line}"
    );
    assert!(
        !transcript.contains(VALUE_CANARY),
        "a header value must never reach the diagnostic output: {transcript}"
    );
    assert!(
        !transcript.contains(CANARY_FRAGMENT),
        "no fragment of a header value may reach the diagnostic output: {transcript}"
    );
    Ok(())
}

/// The diagnostic is emitted by the refusal branch, not by every request, and
/// it does not disturb the malformed-request accounting.
///
/// Non-vacuous because a per-request log would emit a line for each of the
/// admitted requests below, and because the refusal that follows is asserted to
/// produce exactly one line in the same process — so the contrast is observed
/// against a log format already proven to be emitted.
#[test]
fn an_admitted_request_emits_no_refusal_diagnostic() -> Result<(), String> {
    let mut gateway = GatewayProcess::spawn()?;
    gateway.await_ready()?;
    let baseline = gateway.await_diagnostics(|_| false);
    assert!(
        baseline.is_empty(),
        "a healthy admission transcript starts empty: {baseline}"
    );

    for _ in 0..3 {
        assert_eq!(call(gateway.address(), &allowed_headers())?, 200);
    }
    assert_eq!(call(gateway.address(), &refused_headers())?, 400);
    let observed = gateway.await_diagnostics(|text| naming_lines(text) > 0);

    // Exactly one refusal line total, for the one refused request, even though
    // four requests were served.
    assert_eq!(
        naming_lines(&observed),
        1,
        "only the refused request may be named: {observed}"
    );
    assert!(
        !observed.contains(VALUE_CANARY) && !observed.contains(CANARY_FRAGMENT),
        "a header value must never reach the diagnostic output: {observed}"
    );
    Ok(())
}

fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);
    Ok(port)
}

fn drain(mut reader: ChildStderr, sink: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 2048];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let Ok(mut sink) = sink.lock() else {
                        return;
                    };
                    sink.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if sink.len() > DIAGNOSTIC_LIMIT {
                        let cut = sink.len() - DIAGNOSTIC_LIMIT;
                        *sink = sink.chars().skip(cut).collect();
                    }
                }
            }
        }
    })
}
