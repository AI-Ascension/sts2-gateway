// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;
use sts2_gateway::{
    ProfileBaseline, SaveProfileForwardRequest, SaveProfileForwardResponse,
    SaveProfileForwardingPort, SaveProfileId, SaveProfileRoute, SaveProfileStatus,
    SaveProfileTransportFault, UserDataDescriptor,
};

use super::http::{MAX_RESPONSE_BYTES, ReadError, read_response_with_limit, write_request};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(crate) struct HttpSaveProfileForwarder {
    mod_address: String,
    mod_token: String,
}

impl HttpSaveProfileForwarder {
    pub(crate) fn new(mod_address: &str, mod_token: &str) -> Self {
        Self {
            mod_address: mod_address.to_owned(),
            mod_token: mod_token.to_owned(),
        }
    }

    fn exchange(
        &self,
        request: &SaveProfileForwardRequest,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<super::http::HttpResponse, SaveProfileTransportFault> {
        let address = self
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| SaveProfileTransportFault::UnavailableBeforeWrite)?;
        if !address.ip().is_loopback() {
            return Err(SaveProfileTransportFault::UnavailableBeforeWrite);
        }
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .map_err(|_| SaveProfileTransportFault::UnavailableBeforeWrite)?;
        let expires = Instant::now() + EXCHANGE_TIMEOUT;
        let headers = headers(
            request,
            &self.mod_address,
            &self.mod_token,
            body.len(),
            !body.is_empty(),
        );
        write_request(&mut stream, method, path, &headers, body, expires)
            .map_err(|_| SaveProfileTransportFault::DisconnectedAfterWrite)?;
        read_response_with_limit(&mut stream, expires, MAX_RESPONSE_BYTES).map_err(map_read_error)
    }

    fn forward_route(
        &self,
        request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
        let (method, path) = route_target(request.route, &request.operation_id);
        let response = self.exchange(&request, method, &path, &request.body)?;
        decode_response(request, response)
    }
}

impl SaveProfileForwardingPort for HttpSaveProfileForwarder {
    fn forward(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
        self.forward_route(request)
    }

    fn lookup(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<Option<SaveProfileForwardResponse>, SaveProfileTransportFault> {
        let path = format!("/api/v1/save-profile/operations/{}", request.operation_id);
        let wire_request = request_for_lookup(&request);
        let response = self.exchange(&wire_request, "GET", &path, &[])?;
        if response.status == 404 {
            return Ok(None);
        }
        decode_lookup_response(request, response).map(Some)
    }
}

fn request_for_lookup(request: &SaveProfileForwardRequest) -> SaveProfileForwardRequest {
    let mut wire_request = request.clone();
    wire_request.route = SaveProfileRoute::Lookup;
    wire_request
}

fn decode_lookup_response(
    request: SaveProfileForwardRequest,
    response: super::http::HttpResponse,
) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
    let operation_route = request.route;
    let mut wire_request = request;
    wire_request.route = SaveProfileRoute::Lookup;
    let mut decoded = decode_response(wire_request, response)?;
    decoded.route = operation_route;
    Ok(decoded)
}

fn route_target(route: SaveProfileRoute, operation_id: &str) -> (&'static str, String) {
    match route {
        SaveProfileRoute::List => ("GET", String::from("/api/v1/save-profiles")),
        SaveProfileRoute::Current => ("GET", String::from("/api/v1/save-profile/current")),
        SaveProfileRoute::Select => ("POST", String::from("/api/v1/save-profile/select")),
        SaveProfileRoute::CreateDisposable => (
            "POST",
            String::from("/api/v1/save-profile/create-disposable"),
        ),
        SaveProfileRoute::Lookup => (
            "GET",
            format!("/api/v1/save-profile/operations/{operation_id}"),
        ),
    }
}

fn headers(
    request: &SaveProfileForwardRequest,
    address: &str,
    token: &str,
    body_len: usize,
    json_body: bool,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        (String::from("authorization"), format!("Bearer {token}")),
        (String::from("host"), address.to_owned()),
        (String::from("content-length"), body_len.to_string()),
        (
            String::from("x-sts2-instance-id"),
            request.context.instance_id.clone(),
        ),
        (
            String::from("x-sts2-caller-id"),
            request.context.caller_id.clone(),
        ),
        (
            String::from("x-sts2-session-id"),
            request.context.session_id.clone(),
        ),
        (
            String::from("x-sts2-lease-id"),
            request.context.lease_id.clone(),
        ),
        (
            String::from("x-sts2-lease-epoch"),
            request.context.lease_epoch.to_string(),
        ),
        (
            String::from("x-sts2-correlation-id"),
            request.context.correlation_id.clone(),
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

fn decode_response(
    request: SaveProfileForwardRequest,
    response: super::http::HttpResponse,
) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(SaveProfileTransportFault::MalformedResponse);
    }
    let value = super::strict_json::parse(&response.body)
        .map_err(|_| SaveProfileTransportFault::MalformedResponse)?;
    if !response_identity_matches(&value, &request) {
        return Err(SaveProfileTransportFault::MalformedResponse);
    }
    let status = response_status(response.status, &value);
    let baseline = parse_baseline(value.get("baseline"))?;
    let profile_id = parse_profile_id(
        value
            .get("profile_id")
            .or_else(|| value.get("save_profile_id")),
    )?;
    let user_data = parse_user_data(value.get("user_data"))?;
    Ok(SaveProfileForwardResponse {
        operation_id: request.operation_id,
        context: request.context,
        route: request.route,
        status,
        body: response.body,
        profile_id,
        baseline,
        user_data,
        reason: value
            .get("error_code")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn response_identity_matches(value: &Value, request: &SaveProfileForwardRequest) -> bool {
    for (field, expected) in [
        ("instance_id", request.context.instance_id.as_str()),
        ("caller_id", request.context.caller_id.as_str()),
        ("session_id", request.context.session_id.as_str()),
        ("lease_id", request.context.lease_id.as_str()),
        ("correlation_id", request.context.correlation_id.as_str()),
    ] {
        if value.get(field).and_then(Value::as_str) != Some(expected) {
            return false;
        }
    }
    value.get("lease_epoch").and_then(Value::as_u64) == Some(request.context.lease_epoch)
        && value.get("operation_id").and_then(Value::as_str) == Some(request.operation_id.as_str())
}

fn response_status(http_status: u16, value: &Value) -> SaveProfileStatus {
    match value.get("status").and_then(Value::as_str) {
        Some("accepted") => SaveProfileStatus::Accepted,
        Some("unknown") => SaveProfileStatus::Unknown,
        Some("blocked") => SaveProfileStatus::Blocked,
        Some("cancelled") => SaveProfileStatus::Cancelled,
        Some("rejected") => SaveProfileStatus::Rejected,
        Some("settled" | "created" | "selected") => SaveProfileStatus::Settled,
        _ if (200..300).contains(&http_status) => SaveProfileStatus::Settled,
        _ => SaveProfileStatus::Rejected,
    }
}

fn parse_baseline(
    value: Option<&Value>,
) -> Result<Option<ProfileBaseline>, SaveProfileTransportFault> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| SaveProfileTransportFault::MalformedResponse)
}

fn parse_profile_id(
    value: Option<&Value>,
) -> Result<Option<SaveProfileId>, SaveProfileTransportFault> {
    let Some(value) = value.and_then(Value::as_str) else {
        return Ok(None);
    };
    SaveProfileId::try_new(value.to_owned())
        .map(Some)
        .map_err(|_| SaveProfileTransportFault::MalformedResponse)
}

fn parse_user_data(
    value: Option<&Value>,
) -> Result<Option<UserDataDescriptor>, SaveProfileTransportFault> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let descriptor: UserDataDescriptor = serde_json::from_value(value.clone())
        .map_err(|_| SaveProfileTransportFault::MalformedResponse)?;
    descriptor
        .validate()
        .map_err(|_| SaveProfileTransportFault::MalformedResponse)?;
    Ok(Some(descriptor))
}

fn map_read_error(error: ReadError) -> SaveProfileTransportFault {
    match error {
        ReadError::Timeout => SaveProfileTransportFault::TimeoutAfterWrite,
        ReadError::Malformed | ReadError::Oversized => SaveProfileTransportFault::MalformedResponse,
        ReadError::Unavailable => SaveProfileTransportFault::DisconnectedAfterWrite,
    }
}

#[cfg(test)]
#[path = "save_profile_forwarder_tests.rs"]
mod tests;
