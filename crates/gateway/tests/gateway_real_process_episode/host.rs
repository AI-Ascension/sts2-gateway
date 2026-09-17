// SPDX-License-Identifier: MIT

//! The signed host sideband the served gateway drives.
//!
//! The gateway talks to the host over `POST /api/v1/runtime/recovery` with raw
//! JSON frames, and authenticates the host lease-control acknowledgments with
//! an HMAC over an HCJ1 canonicalization of the frame. This fake terminates
//! that hop on a real loopback listener so the assertions observe what the
//! *served* gateway actually sent, in order.
//!
//! It answers only the three kinds a single short episode can require:
//! `host_fence_request`, `lease_install_request`, and `lease_revoke_request`.
//! The thirty-second lease policy outlives every test here, so no renewal is
//! ever needed; an unexpected kind is recorded and refused rather than
//! silently satisfied.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST};
use uuid::Uuid;

use super::gateway::{CALLER, HOST_LEASE_KEY, HOST_PRINCIPAL};
use super::host_crypto::proof_for;
use super::wire::timestamp;

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const DRAIN_DEADLINE: Duration = Duration::from_secs(2);
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The signed host fake, listening on an ephemeral loopback port.
pub(crate) struct HostFake {
    address: String,
    listener: Arc<TcpListener>,
    stop: Arc<AtomicBool>,
    in_flight: Arc<AtomicUsize>,
    observed: Arc<Mutex<Vec<&'static str>>>,
    accept: Option<std::thread::JoinHandle<()>>,
    handlers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

impl HostFake {
    pub(crate) fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("failed to bind the host fake: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("failed to poll the host fake listener: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to read the host fake address: {error}"))?
            .to_string();

        let listener = Arc::new(listener);
        let stop = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let handlers = Arc::new(Mutex::new(Vec::new()));

        let accept = {
            let listener = Arc::clone(&listener);
            let stop = Arc::clone(&stop);
            let in_flight = Arc::clone(&in_flight);
            let observed = Arc::clone(&observed);
            let handlers = Arc::clone(&handlers);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            in_flight.fetch_add(1, Ordering::SeqCst);
                            let in_flight = Arc::clone(&in_flight);
                            let observed = Arc::clone(&observed);
                            let handler = std::thread::spawn(move || {
                                serve(stream, &observed);
                                in_flight.fetch_sub(1, Ordering::SeqCst);
                            });
                            if let Ok(mut handlers) = handlers.lock() {
                                handlers.push(handler);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(ACCEPT_POLL_INTERVAL);
                        }
                        Err(_) => return,
                    }
                }
            })
        };

        Ok(Self {
            address,
            listener,
            stop,
            in_flight,
            observed,
            accept: Some(accept),
            handlers,
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// The kinds the served gateway sent, in arrival order. Blocks briefly for
    /// any in-flight request so a read never races a handler.
    pub(crate) fn observed(&self) -> Vec<&'static str> {
        let deadline = Instant::now() + DRAIN_DEADLINE;
        while self.in_flight.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(ACCEPT_POLL_INTERVAL);
        }
        self.observed
            .lock()
            .map(|kinds| kinds.clone())
            .unwrap_or_default()
    }
}

impl Drop for HostFake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        if let Ok(mut handlers) = self.handlers.lock() {
            for handler in handlers.drain(..) {
                let _ = handler.join();
            }
        }
        let _ = self.listener.set_nonblocking(true);
    }
}

fn serve(mut stream: TcpStream, observed: &Mutex<Vec<&'static str>>) {
    let _ = stream.set_read_timeout(Some(EXCHANGE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(EXCHANGE_TIMEOUT));
    let Some(body) = read_request(&mut stream) else {
        return;
    };
    let Ok(frame) = serde_json::from_slice::<Value>(&body) else {
        let _ = write_json(&mut stream, &json!({"error": "unparsed_frame"}));
        return;
    };
    let kind = frame["kind"].as_str().unwrap_or_default();
    if let Ok(mut observed) = observed.lock() {
        observed.push(match kind {
            "host_fence_request" => "host_fence_request",
            "lease_install_request" => "lease_install_request",
            "lease_revoke_request" => "lease_revoke_request",
            "lease_renew_request" => "lease_renew_request",
            _ => "unrecognized_request",
        });
    }
    let response = match kind {
        "host_fence_request" => host_fence_response(&frame),
        "lease_install_request" => lease_ack(&frame, LeaseAckKind::Install),
        "lease_revoke_request" => lease_ack(&frame, LeaseAckKind::Revoke),
        _ => json!({"error": "unhandled_kind", "kind": kind}),
    };
    let _ = write_json(&mut stream, &response);
}

/// Reads one loopback HTTP/1.1 request and returns its body.
fn read_request(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    if !request_line.starts_with("POST ") {
        return None;
    }
    let mut content_length = None;
    let mut consumed = request_line.len();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).ok()?;
        consumed += read;
        if read == 0 || consumed > MAX_HEADER_BYTES {
            return None;
        }
        let trimmed = line.trim_end_matches("\r\n");
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let length = content_length?;
    if length > MAX_BODY_BYTES {
        return None;
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_json(stream: &mut TcpStream, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&body)?;
    stream.flush()
}

/// The host fence acknowledgment. Every identity is copied from the boot the
/// gateway presented, which is what makes `same_boot_fence` accept it.
fn host_fence_response(frame: &Value) -> Value {
    let boot = frame["payload"]["boot"].clone();
    let fence = json!({
        "host_fence_id": Uuid::new_v4().to_string(),
        "deployment_id": boot["deployment_id"],
        "instance_id": boot["instance_id"],
        "instance_incarnation": boot["instance_incarnation"],
        "boot_id": boot["boot_id"],
        "authority_generation": boot["authority_generation"],
        "fence_generation": 1,
        "created_at": timestamp(),
    });
    json!({
        "contract": sts2_gateway::RECOVERY_CONTRACT,
        "schema_digest": sts2_gateway::RECOVERY_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": frame["correlation_id"],
        "sent_at": timestamp(),
        "actor": {"principal_id": CALLER, "role": "host"},
        "auth": {"principal_id": CALLER, "capability": "host_fence", "proof": Value::Null},
        "kind": "host_fence_response",
        "payload": {
            "result": {"status": "FENCE_ACCEPTED", "retryable": false, "retry_after_seconds": Value::Null},
            "fence": fence,
        },
    })
}

#[derive(Clone, Copy)]
enum LeaseAckKind {
    Install,
    Revoke,
}

impl LeaseAckKind {
    const fn request_name(self) -> &'static str {
        match self {
            Self::Install => "lease_install_request",
            Self::Revoke => "lease_revoke_request",
        }
    }

    const fn response_name(self) -> &'static str {
        match self {
            Self::Install => "lease_install_response",
            Self::Revoke => "lease_revoke_response",
        }
    }

    const fn capability(self) -> &'static str {
        match self {
            Self::Install => "lease_install",
            Self::Revoke => "lease_revoke",
        }
    }

    const fn domain(self) -> &'static str {
        match self {
            Self::Install => "host-lease-control/v1/lease-install-ack",
            Self::Revoke => "host-lease-control/v1/lease-revoke-ack",
        }
    }

    const fn status(self) -> &'static str {
        match self {
            Self::Install => "INSTALLED",
            Self::Revoke => "REVOKED",
        }
    }
}

/// The signed host lease-control acknowledgment.
///
/// An install binds the acknowledgment to the grant's expiry; a revoke reports
/// no expiry at all, which is what `validate_ack` requires of each.
fn lease_ack(frame: &Value, kind: LeaseAckKind) -> Value {
    debug_assert_eq!(
        frame["kind"].as_str(),
        Some(kind.request_name()),
        "acknowledgment kind must match the request"
    );
    let payload = &frame["payload"];
    let grant = &payload["grant"];
    let lease = &grant["lease"];
    let expires_at = match kind {
        LeaseAckKind::Install => lease["expires_at"].clone(),
        LeaseAckKind::Revoke => Value::Null,
    };
    let mut ack = json!({
        "contract": HOST_LEASE_CONTROL_CONTRACT,
        "schema_digest": HOST_LEASE_CONTROL_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": frame["correlation_id"],
        "sent_at": frame["sent_at"],
        "actor": {"principal_id": HOST_PRINCIPAL, "role": "host"},
        "auth": {"principal_id": HOST_PRINCIPAL, "capability": kind.capability(), "proof": Value::Null},
        "kind": kind.response_name(),
        "payload": {
            "ack": {
                "result": {
                    "status": kind.status(),
                    "retryable": false,
                    "retry_after_seconds": Value::Null,
                },
                "installation_id": payload["installation_id"],
                "grant_digest": payload["grant_digest"],
                "boot_id": grant["boot"]["boot_id"],
                "instance_incarnation": grant["boot"]["instance_incarnation"],
                "host_fence_id": grant["fence"]["host_fence_id"],
                "fence_generation": grant["fence"]["fence_generation"],
                "lease_id": lease["lease_id"],
                "lease_epoch": lease["lease_epoch"],
                "host_install_generation": 1,
                "recorded_at": frame["sent_at"],
                "renew_sequence": Value::Null,
                "expires_at": expires_at,
            },
        },
    });
    ack["auth"]["proof"] = Value::String(proof_for(&ack, kind.domain(), &HOST_LEASE_KEY));
    ack
}
