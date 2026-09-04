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
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde_json::to_vec;
    use sts2_gateway::{
        RuntimeV2Action, RuntimeV2CombatPhase, RuntimeV2ForwardingPort, RuntimeV2Message,
        RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation, RuntimeV2Status,
    };

    use super::{HttpRuntimeV2Forwarder, RuntimeV2TransportFault};

    fn state_request() -> RuntimeV2Message {
        RuntimeV2Message::state_request(
            RuntimeV2Metadata::new(),
            "corr-state",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            4,
        )
    }

    fn state_response() -> RuntimeV2Message {
        RuntimeV2Message::state_response(
            RuntimeV2Metadata::new(),
            "corr-state",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
        )
    }

    fn http_response(message: &RuntimeV2Message) -> Result<Vec<u8>, String> {
        let body = to_vec(message).map_err(|error| error.to_string())?;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut response = header.into_bytes();
        response.extend_from_slice(&body);
        Ok(response)
    }

    fn serve_once(
        listener: TcpListener,
        response: Vec<u8>,
    ) -> thread::JoinHandle<Result<String, String>> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    return Err(String::from("client closed before request headers"));
                }
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
                if bytes.len() > 8 * 1024 {
                    return Err(String::from("request headers exceeded test bound"));
                }
            }
            stream
                .write_all(&response)
                .map_err(|error| error.to_string())?;
            String::from_utf8(bytes).map_err(|error| error.to_string())
        })
    }

    #[test]
    fn state_forwarding_uses_the_v2_route_and_fenced_headers() -> Result<(), String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let response = http_response(&state_response())?;
        let server = serve_once(listener, response);
        let mut forwarder = HttpRuntimeV2Forwarder::new(
            &address.to_string(),
            "mod-token",
            "instance-1",
            "caller-1",
            "session-1",
            "lease-1",
            1,
        );

        let result = forwarder
            .forward_state(state_request())
            .map_err(|error| format!("state forwarding failed: {error:?}"))?;
        assert_eq!(result.kind, RuntimeV2MessageKind::StateResponse);
        assert_eq!(result.observation.map(|value| value.generation), Some(4));
        let request = server
            .join()
            .map_err(|_| String::from("forwarder test server panicked"))??;
        if !request.starts_with("GET /api/v2/runtime/state HTTP/1.1\r\n")
            || !request.contains("Authorization: Bearer mod-token\r\n")
            || !request.contains("x-sts2-instance-id: instance-1\r\n")
            || !request.contains("x-sts2-correlation-id: corr-state\r\n")
        {
            return Err(String::from(
                "state request omitted its fixed v2 route or fence",
            ));
        }
        Ok(())
    }

    #[test]
    fn action_forwarding_is_single_request_and_decodes_a_settled_result() -> Result<(), String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let request = RuntimeV2Message::action_request(
            RuntimeV2Metadata::new(),
            "corr-action",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            4,
            "op-1",
            RuntimeV2Action::end_turn(),
        );
        let response = RuntimeV2Message::result(
            RuntimeV2Metadata::new(),
            "corr-action",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            5,
            "op-1",
            RuntimeV2Action::end_turn(),
            RuntimeV2Status::Settled,
            Some(RuntimeV2Observation::new(
                RuntimeV2CombatPhase::PlayerTurn,
                3,
                true,
                5,
            )),
            None,
            Some(sts2_gateway::RuntimeV2EffectWitness::turn_end_settled(5)),
            RuntimeV2MessageKind::ActionResponse,
        );
        let server = serve_once(listener, http_response(&response)?);
        let mut forwarder = HttpRuntimeV2Forwarder::new(
            &address.to_string(),
            "mod-token",
            "instance-1",
            "caller-1",
            "session-1",
            "lease-1",
            1,
        );
        let decoded = forwarder
            .forward_runtime_v2(sts2_gateway::RuntimeV2ForwardRequest::new(request))
            .map_err(|error| format!("action forwarding failed: {error:?}"))?;
        assert_eq!(decoded.status, Some(RuntimeV2Status::Settled));
        let wire_request = server
            .join()
            .map_err(|_| String::from("forwarder test server panicked"))??;
        if !wire_request.starts_with("POST /api/v2/runtime/action HTTP/1.1\r\n") {
            return Err(String::from("action request used an unexpected route"));
        }
        Ok(())
    }

    #[test]
    fn unreachable_mod_is_a_pre_write_unavailable_fault() -> Result<(), String> {
        let mut forwarder = HttpRuntimeV2Forwarder::new(
            "127.0.0.1:1",
            "mod-token",
            "instance-1",
            "caller-1",
            "session-1",
            "lease-1",
            1,
        );
        assert_eq!(
            forwarder.forward_state(state_request()),
            Err(RuntimeV2TransportFault::UnavailableBeforeWrite)
        );
        Ok(())
    }
}
