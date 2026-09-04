// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::super::http::{
    HttpResponse, MAX_RESPONSE_BYTES, ReadError, read_response, write_request,
};

const STATE_PATH: &str = "/api/v3/runtime/state";
const ACTION_PATH: &str = "/api/v3/runtime/action";
const OPERATIONS_PATH: &str = "/api/v3/runtime/operations/";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV3GameplayTransportError {
    Unavailable,
    Timeout,
    Malformed,
    Uncertain,
}

#[derive(Clone, Debug)]
pub(crate) struct HttpRuntimeV3GameplayForwarder {
    mod_address: String,
    mod_token: String,
    instance_id: String,
    caller_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl HttpRuntimeV3GameplayForwarder {
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

    pub(crate) fn forward_state(
        &self,
        correlation_id: &str,
    ) -> Result<HttpResponse, RuntimeV3GameplayTransportError> {
        self.exchange("GET", STATE_PATH, correlation_id, &[])
    }

    pub(crate) fn forward_action(
        &self,
        correlation_id: &str,
        body: &[u8],
    ) -> Result<HttpResponse, RuntimeV3GameplayTransportError> {
        self.exchange("POST", ACTION_PATH, correlation_id, body)
    }

    pub(crate) fn forward_operation(
        &self,
        correlation_id: &str,
        operation_id: &str,
    ) -> Result<HttpResponse, RuntimeV3GameplayTransportError> {
        let path = format!("{OPERATIONS_PATH}{operation_id}");
        self.exchange("GET", &path, correlation_id, &[])
    }

    fn exchange(
        &self,
        method: &str,
        path: &str,
        correlation_id: &str,
        body: &[u8],
    ) -> Result<HttpResponse, RuntimeV3GameplayTransportError> {
        let address = self
            .mod_address
            .to_socket_addrs()
            .map_err(|_| RuntimeV3GameplayTransportError::Unavailable)?
            .next()
            .ok_or(RuntimeV3GameplayTransportError::Unavailable)?;
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|_| RuntimeV3GameplayTransportError::Unavailable)?;
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .map_err(|_| RuntimeV3GameplayTransportError::Unavailable)?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(|_| RuntimeV3GameplayTransportError::Unavailable)?;
        let headers = BTreeMap::from([
            (
                String::from("Authorization"),
                format!("Bearer {}", self.mod_token),
            ),
            (String::from("Host"), self.mod_address.clone()),
            (String::from("Content-Length"), body.len().to_string()),
            (
                String::from("Content-Type"),
                String::from("application/json"),
            ),
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
                correlation_id.to_owned(),
            ),
        ]);
        write_request(&mut stream, method, path, &headers, body)
            .map_err(|_| RuntimeV3GameplayTransportError::Uncertain)?;
        read_response(&mut stream).map_err(map_read_error)
    }
}

pub(crate) fn response_body_is_bounded(response: &HttpResponse) -> bool {
    response.body.len() <= MAX_RESPONSE_BYTES
}

fn map_read_error(error: ReadError) -> RuntimeV3GameplayTransportError {
    match error {
        ReadError::Timeout => RuntimeV3GameplayTransportError::Timeout,
        ReadError::Malformed | ReadError::Oversized => RuntimeV3GameplayTransportError::Malformed,
        ReadError::Unavailable => RuntimeV3GameplayTransportError::Uncertain,
    }
}
