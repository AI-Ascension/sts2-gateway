// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};
use sts2_gateway::{
    RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2ForwardRequest, RuntimeV2ForwardingPort,
    RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError, RuntimeV2Message,
    RuntimeV2Observation, RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault,
};

use super::http::{
    HttpRequest, HttpResponse, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, ReadError, read_request,
    read_response, write_request, write_response,
};
use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;
use super::runtime_v3_gameplay_forwarder::{
    RuntimeV3GameplayForwardError, RuntimeV3GameplayForwarder,
};

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:15525";
const DEFAULT_MOD_ADDRESS: &str = "127.0.0.1:15526";

pub(crate) struct RuntimeService {
    config: RuntimeConfig,
    lease_active: bool,
    runtime_v2: RuntimeV2Ledger<UnconfiguredRuntimeV2Forwarder>,
    runtime_v3: RuntimeV3GameplayForwarder,
}

struct RuntimeConfig {
    listen_address: String,
    mod_address: String,
    gateway_token: String,
    mod_token: String,
    instance_id: String,
    caller_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl RuntimeService {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let config = RuntimeConfig::from_environment()?;
        let binding = RuntimeV2Binding::new(
            &config.instance_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
            RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
        )
        .map_err(|error| format!("Runtime-v2 binding is invalid: {error}"))?;
        let runtime_v2 = RuntimeV2Ledger::new(
            RuntimeV2LedgerConfig::new(8),
            binding,
            UnconfiguredRuntimeV2Forwarder,
        )
        .map_err(|error| format!("Runtime-v2 ledger is invalid: {error}"))?;
        Ok(Self {
            config,
            lease_active: false,
            runtime_v2,
            runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        })
    }

    pub(crate) fn run(mut self) -> Result<(), String> {
        let listener = TcpListener::bind(&self.config.listen_address)
            .map_err(|error| format!("gateway bind failed: {error}"))?;
        println!(
            "sts2-gateway runtime listening on {} for instance {}",
            self.config.listen_address, self.config.instance_id
        );
        for stream in listener.incoming() {
            let mut stream = stream.map_err(|error| format!("gateway accept failed: {error}"))?;
            if let Err(error) = self.handle_connection(&mut stream) {
                let _ = write_response(&mut stream, 500, &json_error("gateway_internal_error"));
                eprintln!("gateway connection failed: {error}");
            }
        }
        Ok(())
    }

    fn handle_connection(&mut self, stream: &mut TcpStream) -> Result<(), String> {
        let request = match read_request(stream) {
            Ok(request) => request,
            Err(status) => {
                write_response(stream, status, &json_error("malformed_request"))
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
        };
        let (status, body) = self.handle_request(&request);
        write_response(stream, status, &body).map_err(|error| error.to_string())
    }

    fn handle_request(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if !headers_are_allowed(&request.headers) {
            return (400, json_error("unsupported_header"));
        }
        if !self.has_gateway_token(request) {
            return (401, json_error("unauthorized"));
        }
        if let Some(route) = RuntimeV3GameplayRoute::parse(
            request.method.as_str(),
            request.path.as_str(),
            self.config.instance_id.as_str(),
        ) {
            return self.runtime_v3_request(request, route);
        }
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/health/ready") if request.body.is_empty() => self.health(),
            ("POST", "/v1/sessions/allocate")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.allocate(&request.body)
            }
            ("GET", path) if path == self.state_path() && request.body.is_empty() => {
                self.relay_data(request, "GET", "/api/v1/runtime/state", &[])
            }
            ("POST", path) if path == self.action_path() && request.content_type_is_json() => {
                if request.body.is_empty() {
                    (400, json_error("action_body_required"))
                } else {
                    self.relay_data(request, "POST", "/api/v1/runtime/action", &request.body)
                }
            }
            ("POST", path)
                if path == self.runtime_v2_action_path() && request.content_type_is_json() =>
            {
                self.runtime_v2_action(request)
            }
            ("GET", path) if path == self.runtime_v2_state_path() && request.body.is_empty() => {
                self.runtime_v2_state(request)
            }
            ("GET", path) if request.body.is_empty() => {
                let Some(operation_id) = self.runtime_v2_operation_id(path) else {
                    return (404, json_error("route_not_found"));
                };
                self.runtime_v2_reconcile(request, operation_id)
            }
            ("POST", path) if path == self.release_path() && request.body.is_empty() => {
                self.release(request)
            }
            _ => (404, json_error("route_not_found")),
        }
    }

    fn runtime_v3_request(
        &mut self,
        request: &HttpRequest,
        route: RuntimeV3GameplayRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if let Err(error) = self.runtime_v3.validate_request(route, &request.body) {
            return (
                runtime_v3_request_status(error),
                json_error(runtime_v3_error_code(error)),
            );
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod(
            if route.is_post() { "POST" } else { "GET" },
            route.downstream_path(),
            &request.body,
            correlation,
        ) {
            Ok(response) => match self.runtime_v3.validate_response(&response.body) {
                Ok(()) => (response.status, response.body),
                Err(error) => (502, json_error(runtime_v3_error_code(error))),
            },
            Err(status) => (status, json_error("runtime_v3_downstream_unavailable")),
        }
    }

    fn health(&self) -> (u16, Vec<u8>) {
        match self.forward_mod("GET", "/health/ready", &[], None) {
            Ok(response) if response.status == 200 => (
                200,
                json_bytes(&json!({
                    "status": "ready",
                    "instance_id": self.config.instance_id,
                    "downstream": "ready"
                })),
            ),
            Ok(_) => (503, json_error("downstream_not_ready")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    fn allocate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return (400, json_error("allocation_body_invalid"));
        };
        let Some(object) = value.as_object() else {
            return (400, json_error("allocation_body_invalid"));
        };
        if object.len() != 3
            || object.get("instance_id").and_then(Value::as_str)
                != Some(self.config.instance_id.as_str())
            || object.get("caller_id").and_then(Value::as_str)
                != Some(self.config.caller_id.as_str())
            || object.get("session_id").and_then(Value::as_str)
                != Some(self.config.session_id.as_str())
        {
            return (409, json_error("allocation_identity_rejected"));
        }
        self.lease_active = true;
        (
            200,
            json_bytes(&json!({
                "status": "allocated",
                "instance_id": self.config.instance_id,
                "caller_id": self.config.caller_id,
                "session_id": self.config.session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "transport": "attached-loopback"
            })),
        )
    }

    fn release(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.lease_active = false;
        (
            200,
            json_bytes(&json!({
                "status": "released",
                "instance_id": self.config.instance_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch
            })),
        )
    }

    fn relay_data(
        &mut self,
        request: &HttpRequest,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if body.len() > MAX_BODY_BYTES
            || (!body.is_empty() && serde_json::from_slice::<Value>(body).is_err())
        {
            return (400, json_error("runtime_body_invalid"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod(method, path, body, correlation) {
            Ok(response) if response.body.len() <= MAX_BODY_BYTES => {
                (response.status, response.body)
            }
            Ok(_) => (502, json_error("downstream_response_oversized")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    fn runtime_v2_action(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if request.body.len() > MAX_BODY_BYTES {
            return (413, json_error("runtime_v2_body_oversized"));
        }
        let Ok(message) = serde_json::from_slice::<RuntimeV2Message>(&request.body) else {
            return (400, json_error("runtime_v2_request_invalid"));
        };
        if request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(message.correlation_id.as_str())
        {
            return (409, json_error("runtime_v2_correlation_mismatch"));
        }
        match self.runtime_v2.submit_action(message) {
            Ok(response) => (runtime_v2_status(&response), runtime_v2_bytes(&response)),
            Err(error) => runtime_v2_error(error),
        }
    }

    fn runtime_v2_state(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let state_request = RuntimeV2Message::state_request(
            self.runtime_v2.binding().metadata().clone(),
            correlation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            self.runtime_v2.observation().generation,
        );
        if state_request.validate().is_err() {
            return (500, json_error("runtime_v2_state_request_invalid"));
        }
        (503, runtime_v2_state_unavailable(&state_request))
    }

    fn runtime_v2_reconcile(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_identity(operation_id) {
            return (400, json_error("runtime_v2_operation_invalid"));
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let message = RuntimeV2Message::reconcile_request(
            self.runtime_v2.binding().metadata().clone(),
            correlation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            self.runtime_v2.observation().generation,
            operation_id,
        );
        match self.runtime_v2.reconcile(message) {
            Ok(response) => (runtime_v2_status(&response), runtime_v2_bytes(&response)),
            Err(error) => runtime_v2_error(error),
        }
    }

    fn check_lease(&self, request: &HttpRequest) -> Result<(), (u16, Vec<u8>)> {
        if !self.lease_active {
            return Err((409, json_error("lease_not_active")));
        }
        let expected_epoch = self.config.lease_epoch.to_string();
        let expected = [
            ("x-sts2-instance-id", self.config.instance_id.as_str()),
            ("x-sts2-caller-id", self.config.caller_id.as_str()),
            ("x-sts2-session-id", self.config.session_id.as_str()),
            ("x-sts2-lease-id", self.config.lease_id.as_str()),
            ("x-sts2-lease-epoch", expected_epoch.as_str()),
        ];
        if expected
            .iter()
            .any(|(name, value)| request.headers.get(*name).map(String::as_str) != Some(*value))
        {
            return Err((409, json_error("lease_fence_rejected")));
        }
        let Some(correlation) = request.headers.get("x-sts2-correlation-id") else {
            return Err((400, json_error("correlation_required")));
        };
        if !safe_identity(correlation) {
            return Err((400, json_error("correlation_invalid")));
        }
        Ok(())
    }

    fn forward_mod(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        correlation: Option<&str>,
    ) -> Result<HttpResponse, u16> {
        let address = self
            .config
            .mod_address
            .to_socket_addrs()
            .map_err(|_| 503_u16)?
            .next()
            .ok_or(503_u16)?;
        let mut stream =
            TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|_| 503_u16)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| 503_u16)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| 503_u16)?;
        let mut headers = BTreeMap::new();
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", self.config.mod_token),
        );
        headers.insert(String::from("Host"), self.config.mod_address.clone());
        headers.insert(String::from("Content-Length"), body.len().to_string());
        if !body.is_empty() {
            headers.insert(
                String::from("Content-Type"),
                String::from("application/json"),
            );
        }
        if let Some(correlation) = correlation {
            headers.insert(
                String::from("x-sts2-instance-id"),
                self.config.instance_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-caller-id"),
                self.config.caller_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-session-id"),
                self.config.session_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-lease-id"),
                self.config.lease_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-lease-epoch"),
                self.config.lease_epoch.to_string(),
            );
            headers.insert(
                String::from("x-sts2-correlation-id"),
                correlation.to_owned(),
            );
        }
        write_request(&mut stream, method, path, &headers, body).map_err(|_| 503_u16)?;
        read_response(&mut stream).map_err(read_error_status)
    }

    fn has_gateway_token(&self, request: &HttpRequest) -> bool {
        let expected = format!("Bearer {}", self.config.gateway_token);
        request.headers.get("authorization").map(String::as_str) == Some(expected.as_str())
    }

    fn state_path(&self) -> String {
        format!("/v1/instances/{}/state", self.config.instance_id)
    }

    fn action_path(&self) -> String {
        format!("/v1/instances/{}/action", self.config.instance_id)
    }

    fn release_path(&self) -> String {
        format!("/v1/instances/{}/release", self.config.instance_id)
    }

    fn runtime_v2_action_path(&self) -> String {
        format!("/v2/instances/{}/action", self.config.instance_id)
    }

    fn runtime_v2_state_path(&self) -> String {
        format!("/v2/instances/{}/state", self.config.instance_id)
    }

    fn runtime_v2_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("/v2/instances/{}/operations/", self.config.instance_id);
        path.strip_prefix(&prefix)
            .filter(|operation_id| !operation_id.is_empty() && !operation_id.contains('/'))
    }
}

/// The attached v1 binary has no authorized Runtime-v2 host adapter yet.
/// Keeping this seam explicit makes the v2 routes safe: no guessed host path is contacted.
struct UnconfiguredRuntimeV2Forwarder;

impl RuntimeV2ForwardingPort for UnconfiguredRuntimeV2Forwarder {
    fn forward_runtime_v2(
        &mut self,
        _request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        Err(RuntimeV2TransportFault::UnavailableBeforeWrite)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        _request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        Ok(None)
    }
}

impl RuntimeConfig {
    fn from_environment() -> Result<Self, String> {
        let listen_address = env_or_default("STS2_GATEWAY_ADDR", DEFAULT_LISTEN_ADDRESS)?;
        let mod_address = env_or_default("STS2_MOD_ADDR", DEFAULT_MOD_ADDRESS)?;
        let gateway_token = required("STS2_GATEWAY_TOKEN")?;
        let mod_token = required("STS2_MOD_TOKEN")?;
        let instance_id = env_or_default("STS2_INSTANCE_ID", "instance-1")?;
        let caller_id = env_or_default("STS2_CALLER_ID", "harness")?;
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let lease_id = env_or_default("STS2_LEASE_ID", "lease-1")?;
        let lease_epoch = env_or_default("STS2_LEASE_EPOCH", "1")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?;
        for (name, value) in [
            ("STS2_INSTANCE_ID", &instance_id),
            ("STS2_CALLER_ID", &caller_id),
            ("STS2_SESSION_ID", &session_id),
            ("STS2_LEASE_ID", &lease_id),
        ] {
            if !safe_identity(value) {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        for (name, value) in [
            ("STS2_GATEWAY_TOKEN", &gateway_token),
            ("STS2_MOD_TOKEN", &mod_token),
        ] {
            if value.is_empty()
                || value.len() > 256
                || value.bytes().any(|byte| byte.is_ascii_whitespace())
            {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        Ok(Self {
            listen_address,
            mod_address,
            gateway_token,
            mod_token,
            instance_id,
            caller_id,
            session_id,
            lease_id,
            lease_epoch,
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn headers_are_allowed(headers: &BTreeMap<String, String>) -> bool {
    headers.keys().all(|name| {
        matches!(
            name.as_str(),
            "authorization"
                | "connection"
                | "content-length"
                | "content-type"
                | "host"
                | "x-mcp-request-id"
                | "x-mcp-session-id"
                | "x-sts2-instance-id"
                | "x-sts2-caller-id"
                | "x-sts2-session-id"
                | "x-sts2-lease-id"
                | "x-sts2-lease-epoch"
                | "x-sts2-correlation-id"
        )
    })
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn json_bytes(value: &Value) -> Vec<u8> {
    match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) => b"{\"error_code\":\"serialization_failed\"}".to_vec(),
    }
}

fn json_error(code: &str) -> Vec<u8> {
    json_bytes(&json!({ "error_code": code }))
}

fn runtime_v2_bytes(message: &RuntimeV2Message) -> Vec<u8> {
    match serde_json::to_vec(message) {
        Ok(bytes) => bytes,
        Err(_) => json_error("runtime_v2_serialization_failed"),
    }
}

fn runtime_v2_status(message: &RuntimeV2Message) -> u16 {
    match message.status {
        Some(RuntimeV2Status::Rejected)
            if message.error_code.as_deref() == Some("idempotency_conflict") =>
        {
            409
        }
        Some(RuntimeV2Status::Rejected | RuntimeV2Status::Cancelled | RuntimeV2Status::Unknown) => {
            200
        }
        Some(RuntimeV2Status::Accepted | RuntimeV2Status::Settled) => 200,
        None => 200,
    }
}

fn runtime_v2_state_unavailable(request: &RuntimeV2Message) -> Vec<u8> {
    json_bytes(&json!({
        "status": "unavailable",
        "error_code": "sts2.runtime/state_unavailable",
        "reason": "unconfigured_runtime_v2_forwarder",
        "request": request,
    }))
}

fn runtime_v2_error(error: RuntimeV2LedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RuntimeV2LedgerError::InvalidRequest(_) | RuntimeV2LedgerError::RequestDigest(_) => {
            (400, "runtime_v2_request_invalid")
        }
        RuntimeV2LedgerError::MissingOperationId => (400, "runtime_v2_operation_required"),
        RuntimeV2LedgerError::CapacityExceeded => (429, "runtime_v2_operation_capacity"),
        RuntimeV2LedgerError::OperationNotFound => (404, "runtime_v2_operation_not_found"),
        RuntimeV2LedgerError::OperationInProgress => (409, "runtime_v2_operation_in_progress"),
        RuntimeV2LedgerError::Fence(_) => (409, "runtime_v2_lease_fence_rejected"),
        RuntimeV2LedgerError::StaleGeneration { .. } => (409, "runtime_v2_stale_generation"),
        RuntimeV2LedgerError::ZeroCapacity => (500, "runtime_v2_operation_capacity_invalid"),
    };
    (status, json_error(code))
}

fn read_error_status(error: ReadError) -> u16 {
    match error {
        ReadError::Timeout => 504,
        ReadError::Malformed | ReadError::Oversized => 502,
        ReadError::Unavailable => 503,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use sts2_gateway::{
        RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
        RuntimeV2Observation,
    };

    use super::{HttpRequest, RuntimeConfig, RuntimeService, UnconfiguredRuntimeV2Forwarder};

    fn test_service() -> Result<RuntimeService, String> {
        let config = RuntimeConfig {
            listen_address: String::from("127.0.0.1:15525"),
            mod_address: String::from("127.0.0.1:15526"),
            gateway_token: String::from("gateway-token"),
            mod_token: String::from("mod-token"),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
        };
        let binding = RuntimeV2Binding::new(
            &config.instance_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
            RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
        )
        .map_err(|error| error.to_string())?;
        let runtime_v2 = RuntimeV2Ledger::new(
            RuntimeV2LedgerConfig::new(8),
            binding,
            UnconfiguredRuntimeV2Forwarder,
        )
        .map_err(|error| error.to_string())?;
        Ok(RuntimeService {
            config,
            lease_active: true,
            runtime_v2,
            runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        })
    }

    fn authenticated_request(path: &str) -> HttpRequest {
        let mut headers = BTreeMap::new();
        headers.insert(
            String::from("authorization"),
            String::from("Bearer gateway-token"),
        );
        headers.insert(
            String::from("x-sts2-instance-id"),
            String::from("instance-1"),
        );
        headers.insert(String::from("x-sts2-caller-id"), String::from("harness"));
        headers.insert(String::from("x-sts2-session-id"), String::from("session-1"));
        headers.insert(String::from("x-sts2-lease-id"), String::from("lease-1"));
        headers.insert(String::from("x-sts2-lease-epoch"), String::from("1"));
        headers.insert(
            String::from("x-sts2-correlation-id"),
            String::from("corr-state"),
        );
        HttpRequest {
            method: String::from("GET"),
            path: path.to_owned(),
            headers,
            body: Vec::new(),
        }
    }

    #[test]
    fn state_route_returns_typed_request_and_explicit_unavailable_fallback() -> Result<(), String> {
        let mut service = test_service()?;
        let request = authenticated_request("/v2/instances/instance-1/state");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["status"], "unavailable");
        assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
        assert_eq!(value["reason"], "unconfigured_runtime_v2_forwarder");
        assert_eq!(value["request"]["kind"], "state_request");
        assert_eq!(value["request"]["instance_id"], "instance-1");
        assert_eq!(value["request"]["correlation_id"], "corr-state");
        Ok(())
    }

    #[test]
    fn v2_gets_are_not_arbitrary_proxy_routes() -> Result<(), String> {
        let mut service = test_service()?;
        for path in [
            "/v2/instances/instance-1/state/extra",
            "/v2/instances/instance-1/not-a-proxy",
        ] {
            let (status, _) = service.handle_request(&authenticated_request(path));
            assert_eq!(status, 404, "unexpected route match for {path}");
        }
        Ok(())
    }
}

fn runtime_v3_request_status(error: RuntimeV3GameplayForwardError) -> u16 {
    match error {
        RuntimeV3GameplayForwardError::RequestBodyOversized => 413,
        RuntimeV3GameplayForwardError::RequestBodyRequired
        | RuntimeV3GameplayForwardError::RequestBodyMalformed => 400,
        RuntimeV3GameplayForwardError::ResponseOversized
        | RuntimeV3GameplayForwardError::ResponseMalformed => 502,
    }
}

fn runtime_v3_error_code(error: RuntimeV3GameplayForwardError) -> &'static str {
    match error {
        RuntimeV3GameplayForwardError::RequestBodyRequired => "runtime_v3_body_required",
        RuntimeV3GameplayForwardError::RequestBodyOversized => "runtime_v3_body_oversized",
        RuntimeV3GameplayForwardError::RequestBodyMalformed => "runtime_v3_request_invalid",
        RuntimeV3GameplayForwardError::ResponseOversized => "runtime_v3_response_oversized",
        RuntimeV3GameplayForwardError::ResponseMalformed => "runtime_v3_response_invalid",
    }
}
