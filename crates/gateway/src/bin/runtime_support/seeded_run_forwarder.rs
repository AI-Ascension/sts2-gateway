// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::from_slice;
use sts2_gateway::{
    SeededRunForwardRequest, SeededRunForwardingPort, SeededRunMessage, SeededRunMessageKind,
    SeededRunReceiptRequest, SeededRunTransportFault,
};

use super::http::{MAX_RESPONSE_BYTES, ReadError, read_response_with_limit, write_request};

const START_PATH: &str = "/v2/seeded-run";
const RECEIPT_PREFIX: &str = "/v2/seeded-operations/";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct HttpSeededRunForwarder {
    mod_address: String,
    mod_token: String,
    instance_id: String,
    caller_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl HttpSeededRunForwarder {
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

    fn exchange(
        &self,
        method: &str,
        path: &str,
        message: &SeededRunMessage,
        body: &[u8],
    ) -> Result<super::http::HttpResponse, SeededRunTransportFault> {
        let address = self
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| SeededRunTransportFault::UnavailableBeforeWrite)?;
        if !address.ip().is_loopback() {
            return Err(SeededRunTransportFault::UnavailableBeforeWrite);
        }
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|_| SeededRunTransportFault::UnavailableBeforeWrite)?;
        let expires = Instant::now() + EXCHANGE_TIMEOUT;
        write_request(
            &mut stream,
            method,
            path,
            &self.headers(message, body.len(), !body.is_empty()),
            body,
            expires,
        )
        .map_err(|_| SeededRunTransportFault::Timeout)?;
        read_response_with_limit(&mut stream, expires, MAX_RESPONSE_BYTES).map_err(map_read_error)
    }

    fn headers(
        &self,
        message: &SeededRunMessage,
        body_length: usize,
        json_body: bool,
    ) -> BTreeMap<String, String> {
        let mut headers = BTreeMap::from([
            (
                String::from("authorization"),
                format!("Bearer {}", self.mod_token),
            ),
            (String::from("host"), self.mod_address.clone()),
            (String::from("content-length"), body_length.to_string()),
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
        ]);
        if json_body {
            headers.insert(
                String::from("content-type"),
                String::from("application/json"),
            );
        }
        headers
    }

    fn decode(
        &self,
        response: super::http::HttpResponse,
        request: &SeededRunMessage,
        expected_kind: SeededRunMessageKind,
    ) -> Result<SeededRunMessage, SeededRunTransportFault> {
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(SeededRunTransportFault::MalformedResponse);
        }
        let message = from_slice::<SeededRunMessage>(&response.body)
            .map_err(|_| SeededRunTransportFault::MalformedResponse)?;
        if message.validate().is_err()
            || message.kind != expected_kind
            || !matches_request(request, &message)
        {
            return Err(SeededRunTransportFault::MalformedResponse);
        }
        Ok(message)
    }

    fn decode_receipt(
        &self,
        response: super::http::HttpResponse,
        request: &SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunTransportFault> {
        if response.body.len() > MAX_RESPONSE_BYTES {
            return Err(SeededRunTransportFault::MalformedResponse);
        }
        let message = from_slice::<SeededRunMessage>(&response.body)
            .map_err(|_| SeededRunTransportFault::MalformedResponse)?;
        if message.validate().is_err()
            || message.kind != SeededRunMessageKind::ReconcileResponse
            || message.protocol_version != request.protocol_version
            || message.schema_digest != request.schema_digest
            || message.provenance != request.provenance
            || message.correlation_id != request.correlation_id
            || message.instance_id != request.instance_id
            || message.session_id != request.session_id
            || message.lease_id != request.lease_id
            || message.lease_epoch != request.lease_epoch
            || message.operation_id != request.operation_id
        {
            return Err(SeededRunTransportFault::MalformedResponse);
        }
        Ok(message)
    }
}

impl SeededRunForwardingPort for HttpSeededRunForwarder {
    fn forward_seeded_run(
        &mut self,
        request: SeededRunForwardRequest,
    ) -> Result<SeededRunMessage, SeededRunTransportFault> {
        let message = request.message;
        let body =
            serde_json::to_vec(&message).map_err(|_| SeededRunTransportFault::MalformedResponse)?;
        let response = self.exchange("POST", START_PATH, &message, &body)?;
        self.decode(response, &message, SeededRunMessageKind::StartResponse)
    }

    fn read_seeded_run_receipt(
        &mut self,
        request: SeededRunReceiptRequest,
    ) -> Result<Option<SeededRunMessage>, SeededRunTransportFault> {
        let message = request.message;
        let path = format!("{RECEIPT_PREFIX}{}", request.operation_id);
        let response = self.exchange("GET", &path, &message, &[])?;
        if response.status == 404 {
            return Ok(None);
        }
        self.decode_receipt(response, &message).map(Some)
    }
}

fn matches_request(request: &SeededRunMessage, response: &SeededRunMessage) -> bool {
    request.protocol_version == response.protocol_version
        && request.schema_digest == response.schema_digest
        && request.provenance == response.provenance
        && request.instance_id == response.instance_id
        && request.session_id == response.session_id
        && request.lease_id == response.lease_id
        && request.lease_epoch == response.lease_epoch
        && request.operation_id == response.operation_id
        && request.requested_seed == response.requested_seed
        && request.run_mode == response.run_mode
        && request.context_digest == response.context_digest
        && request.selected_context == response.selected_context
}

fn map_read_error(error: ReadError) -> SeededRunTransportFault {
    match error {
        ReadError::Timeout => SeededRunTransportFault::Timeout,
        ReadError::Malformed | ReadError::Oversized => SeededRunTransportFault::MalformedResponse,
        ReadError::Unavailable => SeededRunTransportFault::Timeout,
    }
}
