// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::from_slice;
use sts2_gateway::{
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2Message, RuntimeV2MessageKind,
    RuntimeV2ReceiptRequest, RuntimeV2TransportFault,
};

use super::http::{HttpResponse, MAX_RESPONSE_BYTES, ReadError, read_response, write_request};

const ACTION_PATH: &str = "/api/v2/runtime/action";
const STATE_PATH: &str = "/api/v2/runtime/state";
const OPERATIONS_PATH: &str = "/api/v2/runtime/operations/";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP forwarding implementation for the authenticated Runtime-v2 mod boundary.
///
/// The forwarder sends one mutation request at most once. A write or response failure after the
/// request is sent is returned as uncertainty so the ledger can reconcile by operation identity;
/// this type never retries an action internally.
#[derive(Clone, Debug)]
pub(crate) struct HttpRuntimeV2Forwarder {
    mod_address: String,
    mod_token: String,
    instance_id: String,
    caller_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl HttpRuntimeV2Forwarder {
    pub(crate) fn new(
        mod_address: &str,
        mod_token: &str,
        instance_id: &str,
        caller_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
    ) -> Self {
        Self {
            mod_address: mod_address.to_owned(),
            mod_token: mod_token.to_owned(),
            instance_id: instance_id.to_owned(),
            caller_id: caller_id.to_owned(),
            session_id: session_id.to_owned(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
        }
    }

    /// Forwards one read-only state request to the mod's v2 state route.
    pub(crate) fn forward_state(
        &mut self,
        request: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        let response = self.exchange("GET", STATE_PATH, &request, false)?;
        self.decode(response, &request, RuntimeV2MessageKind::StateResponse)
    }

    fn exchange(
        &self,
        method: &str,
        path: &str,
        message: &RuntimeV2Message,
        include_body: bool,
    ) -> Result<HttpResponse, RuntimeV2TransportFault> {
        let expires = Instant::now() + EXCHANGE_TIMEOUT;
        let address = self
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| RuntimeV2TransportFault::UnavailableBeforeWrite)?;
        if !address.ip().is_loopback() {
            return Err(RuntimeV2TransportFault::UnavailableBeforeWrite);
        }
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|_| RuntimeV2TransportFault::UnavailableBeforeWrite)?;

        let body = if include_body {
            serde_json::to_vec(message).map_err(|_| RuntimeV2TransportFault::RejectedBeforeWrite)?
        } else {
            Vec::new()
        };
        let headers = self.headers(message, body.len(), include_body);
        write_request(&mut stream, method, path, &headers, &body, expires)
            .map_err(|_| RuntimeV2TransportFault::DisconnectedAfterWrite)?;
        read_response(&mut stream, expires).map_err(map_read_error)
    }

    fn headers(
        &self,
        message: &RuntimeV2Message,
        body_length: usize,
        include_body: bool,
    ) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                String::from("Authorization"),
                format!("Bearer {}", self.mod_token),
            ),
            (String::from("Host"), self.mod_address.clone()),
            (String::from("Content-Length"), body_length.to_string()),
            (String::from("x-sts2-instance-id"), self.instance_id.clone()),
            (String::from("x-sts2-caller-id"), self.caller_id.clone()),
            (String::from("x-sts2-session-id"), self.session_id.clone()),
            (String::from("x-sts2-lease-id"), self.lease_id.clone()),
            (
                String::from("x-sts2-lease-epoch"),
                self.lease_epoch.to_string(),
            ),
            (
                String::from("x-sts2-correlation-id"),
                message.correlation_id.clone(),
            ),
        ])
        .into_iter()
        .chain(include_body.then(|| {
            (
                String::from("Content-Type"),
                String::from("application/json"),
            )
        }))
        .collect()
    }

    fn decode(
        &self,
        response: HttpResponse,
        request: &RuntimeV2Message,
        expected_kind: RuntimeV2MessageKind,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(RuntimeV2TransportFault::MalformedResponse);
        }
        let message = from_slice::<RuntimeV2Message>(&response.body)
            .map_err(|_| classify_invalid_response_status(response.status))?;
        if message.validate().is_err()
            || message.kind != expected_kind
            || !matches_request(request, &message)
        {
            return Err(RuntimeV2TransportFault::MalformedResponse);
        }
        Ok(message)
    }
}

impl RuntimeV2ForwardingPort for HttpRuntimeV2Forwarder {
    fn forward_runtime_v2(
        &mut self,
        request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        let message = request.message().clone();
        let response = self.exchange("POST", ACTION_PATH, &message, true)?;
        self.decode(response, &message, RuntimeV2MessageKind::ActionResponse)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        let message = request.message().clone();
        let path = format!("{OPERATIONS_PATH}{}", request.key().operation_id());
        let response = self.exchange("GET", &path, &message, false)?;
        if response.status == 404 {
            return Ok(None);
        }
        self.decode(response, &message, RuntimeV2MessageKind::ActionResponse)
            .map(Some)
    }
}

fn matches_request(request: &RuntimeV2Message, response: &RuntimeV2Message) -> bool {
    request.protocol_version == response.protocol_version
        && request.schema_digest == response.schema_digest
        && request.provenance == response.provenance
        && request.correlation_id == response.correlation_id
        && request.instance_id == response.instance_id
        && request.session_id == response.session_id
        && request.lease_id == response.lease_id
        && request.lease_epoch == response.lease_epoch
}

fn map_read_error(error: ReadError) -> RuntimeV2TransportFault {
    match error {
        ReadError::Timeout => RuntimeV2TransportFault::TimeoutAfterWrite,
        ReadError::Malformed | ReadError::Oversized => RuntimeV2TransportFault::MalformedResponse,
        ReadError::Unavailable => RuntimeV2TransportFault::DisconnectedAfterWrite,
    }
}

fn classify_invalid_response_status(status: u16) -> RuntimeV2TransportFault {
    match status {
        400 | 409 | 413 | 422 => RuntimeV2TransportFault::RejectedBeforeWrite,
        408 | 504 => RuntimeV2TransportFault::TimeoutAfterWrite,
        _ => RuntimeV2TransportFault::MalformedResponse,
    }
}

#[cfg(test)]
#[path = "forwarder_tests.rs"]
mod tests;
