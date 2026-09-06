// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use sts2_gateway::{MAX_RECOVERY_FRAME_BYTES, RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST};

use super::http::{HttpResponse, ReadError, read_response, write_request};

pub(crate) const HOST_FENCE_PATH: &str = "/v1/recovery/host-fence";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Downstream recovery control transport. The fixed host-fence endpoint is the only operation
/// exposed here; boot and lease identities remain in the closed recovery frame body. In
/// particular, this request never requires the old gameplay lease headers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpRecoveryControlForwarder {
    mod_address: String,
    mod_token: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryControlTransportFault {
    InvalidFrame,
    RequestOversized,
    InvalidConfiguration,
    UnavailableBeforeWrite,
    DisconnectedAfterWrite,
    TimeoutAfterWrite,
    MalformedResponse,
}

impl HttpRecoveryControlForwarder {
    pub(crate) fn new(mod_address: &str, mod_token: &str) -> Self {
        Self {
            mod_address: mod_address.to_owned(),
            mod_token: mod_token.to_owned(),
        }
    }

    /// Sends a validated `host_fence_request` recovery frame to the fixed mod route.
    ///
    /// The method is single-shot: after the request starts writing, every I/O failure is
    /// reported as an after-write uncertainty. Callers must reconcile the host-fence result
    /// through the recovery protocol rather than blindly retrying a mutation-bearing request.
    pub(crate) fn forward_host_fence(
        &self,
        frame: &[u8],
    ) -> Result<HttpResponse, RecoveryControlTransportFault> {
        validate_frame(frame)?;
        if !valid_token(&self.mod_token) {
            return Err(RecoveryControlTransportFault::InvalidConfiguration);
        }
        let address = self
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| RecoveryControlTransportFault::UnavailableBeforeWrite)?;
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(RecoveryControlTransportFault::UnavailableBeforeWrite);
        }
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|_| RecoveryControlTransportFault::UnavailableBeforeWrite)?;
        let headers = BTreeMap::from([
            (
                String::from("Authorization"),
                format!("Bearer {}", self.mod_token),
            ),
            (String::from("Host"), self.mod_address.clone()),
            (String::from("Content-Length"), frame.len().to_string()),
            (
                String::from("Content-Type"),
                String::from("application/json"),
            ),
        ]);
        let expires = Instant::now() + EXCHANGE_TIMEOUT;
        write_request(
            &mut stream,
            "POST",
            HOST_FENCE_PATH,
            &headers,
            frame,
            expires,
        )
        .map_err(|_| RecoveryControlTransportFault::DisconnectedAfterWrite)?;
        read_response(&mut stream, expires).map_err(map_read_error)
    }
}

fn validate_frame(frame: &[u8]) -> Result<(), RecoveryControlTransportFault> {
    if frame.is_empty() {
        return Err(RecoveryControlTransportFault::InvalidFrame);
    }
    if frame.len() > MAX_RECOVERY_FRAME_BYTES {
        return Err(RecoveryControlTransportFault::RequestOversized);
    }
    let value = super::strict_json::parse(frame)
        .map_err(|_| RecoveryControlTransportFault::InvalidFrame)?;
    let object = value
        .as_object()
        .ok_or(RecoveryControlTransportFault::InvalidFrame)?;
    if object.len() != 9
        || value["contract"].as_str() != Some(RECOVERY_CONTRACT)
        || value["schema_digest"].as_str() != Some(RECOVERY_SCHEMA_DIGEST)
        || value["kind"].as_str() != Some("host_fence_request")
        || !value["message_id"].is_string()
        || !value["correlation_id"].is_string()
        || !value["sent_at"].is_string()
        || !valid_actor(&value["actor"])
        || !valid_auth(&value["auth"])
        || !valid_boot_payload(&value["payload"])
    {
        return Err(RecoveryControlTransportFault::InvalidFrame);
    }
    Ok(())
}

fn valid_actor(value: &serde_json::Value) -> bool {
    let Some(actor) = value.as_object() else {
        return false;
    };
    actor.len() == 2
        && actor["principal_id"].is_string()
        && matches!(actor["role"].as_str(), Some("gateway" | "watchdog"))
}

fn valid_auth(value: &serde_json::Value) -> bool {
    let Some(auth) = value.as_object() else {
        return false;
    };
    auth.len() == 3
        && auth["principal_id"].is_string()
        && auth["capability"].as_str() == Some("host_fence")
        && auth["proof"]
            .as_str()
            .is_some_and(|proof| !proof.is_empty())
}

fn valid_boot_payload(value: &serde_json::Value) -> bool {
    let Some(payload) = value.as_object() else {
        return false;
    };
    payload.len() == 1 && payload["boot"].is_object()
}

fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= 256 && token.bytes().all(|byte| !byte.is_ascii_whitespace())
}

fn map_read_error(error: ReadError) -> RecoveryControlTransportFault {
    match error {
        ReadError::Timeout => RecoveryControlTransportFault::TimeoutAfterWrite,
        ReadError::Malformed | ReadError::Oversized => {
            RecoveryControlTransportFault::MalformedResponse
        }
        ReadError::Unavailable => RecoveryControlTransportFault::DisconnectedAfterWrite,
    }
}

#[cfg(test)]
#[path = "recovery_control_tests.rs"]
mod tests;
