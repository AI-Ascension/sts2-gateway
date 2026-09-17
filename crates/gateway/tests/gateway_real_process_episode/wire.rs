// SPDX-License-Identifier: MIT

//! The recovery and gameplay wire the tests speak to the served gateway.
//!
//! The frame builders here are deliberately independent of the gateway
//! binary's own (bin-private) implementations. A test that reused the
//! production encoders could pass while the served process rejected the very
//! same bytes, so the client side is re-derived from the contract instead.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};
use sts2_gateway::{RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST, RUNTIME_V3_SCHEMA_DIGEST};
use uuid::Uuid;

use super::gateway::{CALLER, GATEWAY_TOKEN, INSTANCE, MCP_SESSION, RECOVERY_TOKEN, SESSION};

/// The negotiated repeated-episode profile name (ADR 0033).
pub(crate) const EPISODE_PROFILE: &str = "repeated-episode-lease-v1";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// The four digests a boot authority is bound to. Only the runtime schema
/// digest is meaningful here; the other three are opaque defaults.
pub(crate) fn release_set() -> Value {
    json!({
        "release_digest": "0".repeat(64),
        "config_digest": "0".repeat(64),
        "profile_digest": "0".repeat(64),
        "runtime_v3_schema_digest": RUNTIME_V3_SCHEMA_DIGEST,
    })
}

/// The lease's own context, read back out of the lease the served process
/// issued. Lease identity and epoch differ per episode, so nothing here may be
/// hardcoded.
///
/// A value that lacks the fields yields nulls rather than panicking, which
/// keeps the placeholder call in the test module honest.
pub(crate) fn original_context(lease: &Value) -> Value {
    json!({
        "deployment_id": lease["deployment_id"],
        "instance_id": lease["instance_id"],
        "instance_incarnation": lease["instance_incarnation"],
        "boot_id": lease["boot_id"],
        "authority_generation": lease["authority_generation"],
        "lease_id": lease["lease_id"],
        "lease_epoch": lease["lease_epoch"],
    })
}

/// The `error_code` of a gateway error body.
pub(crate) fn error_code(value: &Value) -> &str {
    value["error_code"].as_str().unwrap_or_default()
}

/// Reads the status of an authenticated `GET`, discarding the body.
pub(crate) fn get_status(address: &str, path: &str, token: &str) -> Result<u16, String> {
    let authorization = format!("Bearer {token}");
    let headers = [("Authorization", authorization.as_str())];
    send(address, "GET", path, &headers, b"").map(|(status, _)| status)
}

/// Sends one closed recovery frame and returns the status and parsed body.
pub(crate) fn call(
    address: &str,
    path: &str,
    kind: &str,
    capability: &str,
    payload: Value,
) -> Result<(u16, Value), String> {
    let body = recovery_frame(kind, capability, payload)?;
    let authorization = format!("Bearer {RECOVERY_TOKEN}");
    let headers = [
        ("Authorization", authorization.as_str()),
        ("Content-Type", "application/json"),
        ("x-sts2-recovery-capability", capability),
    ];
    send(address, "POST", path, &headers, &body)
}

pub(crate) fn host_fence(address: &str, boot: &Value) -> Result<(u16, Value), String> {
    call(
        address,
        "/v1/recovery/host-fence",
        "host_fence_request",
        "host_fence",
        json!({"boot": boot}),
    )
}

pub(crate) fn acquire(address: &str, boot: &Value, fence: &Value) -> Result<(u16, Value), String> {
    call(
        address,
        "/v1/recovery/lease/acquire",
        "lease_acquire_request",
        "lease_acquire",
        json!({"boot": boot, "fence": fence}),
    )
}

/// Completes the episode held by `lease` through the gameplay release route.
///
/// The profile header is the only thing that opts an episode into the
/// repeated-episode contract, so it is passed explicitly rather than defaulted.
pub(crate) fn instance_release(
    address: &str,
    lease: &Value,
    profile: Option<&str>,
) -> Result<(u16, Value), String> {
    let authorization = format!("Bearer {GATEWAY_TOKEN}");
    let correlation = Uuid::new_v4().to_string();
    let epoch = lease["lease_epoch"]
        .as_u64()
        .unwrap_or_default()
        .to_string();
    let mut headers = vec![
        ("Authorization", authorization.as_str()),
        ("Content-Type", "application/json"),
        ("x-sts2-instance-id", INSTANCE),
        ("x-sts2-caller-id", CALLER),
        ("x-sts2-session-id", SESSION),
        ("x-mcp-session-id", MCP_SESSION),
        (
            "x-sts2-lease-id",
            lease["lease_id"].as_str().unwrap_or_default(),
        ),
        ("x-sts2-lease-epoch", epoch.as_str()),
        ("x-sts2-correlation-id", correlation.as_str()),
    ];
    if let Some(profile) = profile {
        headers.push(("x-sts2-episode-profile", profile));
    }
    send(
        address,
        "POST",
        &format!("/v1/instances/{INSTANCE}/release"),
        &headers,
        b"",
    )
}

fn recovery_frame(kind: &str, capability: &str, payload: Value) -> Result<Vec<u8>, String> {
    let frame = json!({
        "contract": RECOVERY_CONTRACT,
        "schema_digest": RECOVERY_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": Uuid::new_v4().to_string(),
        "sent_at": timestamp(),
        "actor": {"principal_id": CALLER, "role": "gateway"},
        "auth": {"principal_id": CALLER, "capability": capability, "proof": Value::Null},
        "kind": kind,
        "payload": payload,
    });
    serde_json::to_vec(&frame)
        .map_err(|error| format!("failed to encode a recovery frame: {error}"))
}

/// One request per connection, `Connection: close`, explicit length. This is
/// the shape the gateway's own request reader expects.
fn send(
    address: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<(u16, Value), String> {
    let target = address
        .parse()
        .map_err(|error| format!("invalid gateway address {address}: {error}"))?;
    let mut stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)
        .map_err(|error| format!("failed to reach the gateway at {address}: {error}"))?;
    stream
        .set_read_timeout(Some(EXCHANGE_TIMEOUT))
        .map_err(|error| format!("failed to bound the gateway read: {error}"))?;
    stream
        .set_write_timeout(Some(EXCHANGE_TIMEOUT))
        .map_err(|error| format!("failed to bound the gateway write: {error}"))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    request.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|error| format!("failed to write the {method} {path} request: {error}"))?;

    let (status, body) = read_response(&mut stream)?;
    Ok((status, parse_body(&body)))
}

fn read_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>), String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            break end;
        }
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(String::from(
                "the gateway response headers exceeded the test bound",
            ));
        }
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("failed to read the gateway response: {error}"))?;
        if read == 0 {
            return Err(String::from(
                "the gateway closed the connection before sending a full response",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    };

    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|error| format!("the gateway response header was not UTF-8: {error}"))?;
    let mut lines = header.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| String::from("the gateway response had no status line"))?;
    let status = status_line
        .split_ascii_whitespace()
        .nth(1)
        .ok_or_else(|| format!("malformed gateway status line: {status_line}"))?
        .parse::<u16>()
        .map_err(|error| format!("malformed gateway status code in {status_line}: {error}"))?;
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().to_owned())
        .ok_or_else(|| String::from("the gateway response carried no content-length"))?
        .parse::<usize>()
        .map_err(|error| format!("malformed gateway content-length: {error}"))?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(String::from("the gateway response exceeded the test bound"));
    }

    let mut body = bytes[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("failed to read the gateway body: {error}"))?;
        if read == 0 {
            return Err(String::from("the gateway closed the connection mid-body"));
        }
        body.extend_from_slice(&buffer[..read]);
    }
    body.truncate(content_length);
    Ok((status, body))
}

fn parse_body(body: &[u8]) -> Value {
    if body.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(body)
        .unwrap_or_else(|_| json!({"unparsed": String::from_utf8_lossy(body)}))
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

/// `YYYY-MM-DDTHH:MM:SS.fffZ`, the only timestamp shape the recovery contract
/// accepts.
pub(crate) fn timestamp() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            elapsed.as_millis().min(u128::from(u64::MAX)) as u64
        });
    timestamp_from_millis(millis)
}

fn timestamp_from_millis(millis: u64) -> String {
    let seconds = millis / 1_000;
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_seconds / 3_600,
        day_seconds / 60 % 60,
        day_seconds % 60,
        millis % 1_000
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}
