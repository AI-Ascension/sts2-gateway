// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use sts2_gateway::MAX_RECOVERY_FRAME_BYTES;

use super::http::{HttpResponse, ReadError, read_response, write_request};
use super::recovery_frame::{RecoveryFrame, RecoveryFrameError, RecoveryKind};

/// The mod-local recovery mux. The closed recovery frame carries the operation kind; the
/// transport path is deliberately fixed so a caller cannot turn this bridge into an arbitrary
/// downstream proxy.
pub(crate) const RECOVERY_CONTROL_PATH: &str = "/api/v1/runtime/recovery";
#[allow(dead_code)]
pub(crate) const HOST_FENCE_PATH: &str = RECOVERY_CONTROL_PATH;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Downstream recovery control transport. Boot, lease, operation, and witness identities remain
/// in the closed recovery frame body. In particular, this request never requires the old gameplay
/// lease headers.
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

    /// Sends a validated recovery request frame to the fixed mod recovery mux.
    ///
    /// The method is single-shot: after the request starts writing, every I/O failure is
    /// reported as an after-write uncertainty. Callers must reconcile the host-fence result
    /// through the recovery protocol rather than blindly retrying a mutation-bearing request.
    pub(crate) fn forward_frame(
        &self,
        kind: RecoveryKind,
        frame: &[u8],
    ) -> Result<HttpResponse, RecoveryControlTransportFault> {
        validate_frame(kind, frame)?;
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
            RECOVERY_CONTROL_PATH,
            &headers,
            frame,
            expires,
        )
        .map_err(|_| RecoveryControlTransportFault::DisconnectedAfterWrite)?;
        read_response(&mut stream, expires).map_err(map_read_error)
    }

    pub(crate) fn forward_host_fence(
        &self,
        frame: &[u8],
    ) -> Result<HttpResponse, RecoveryControlTransportFault> {
        self.forward_frame(RecoveryKind::HostFence, frame)
    }
}

fn validate_frame(kind: RecoveryKind, frame: &[u8]) -> Result<(), RecoveryControlTransportFault> {
    if frame.is_empty() {
        return Err(RecoveryControlTransportFault::InvalidFrame);
    }
    if frame.len() > MAX_RECOVERY_FRAME_BYTES {
        return Err(RecoveryControlTransportFault::RequestOversized);
    }
    RecoveryFrame::parse(frame, kind)
        .map(|_| ())
        .map_err(|error| match error {
            RecoveryFrameError::Oversized => RecoveryControlTransportFault::RequestOversized,
            RecoveryFrameError::Invalid => RecoveryControlTransportFault::InvalidFrame,
        })
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
