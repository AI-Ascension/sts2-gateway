// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

use super::http::{
    HttpRequest, HttpResponse, MAX_BODY_BYTES, ReadError, read_request, read_response,
    write_request, write_response,
};

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:15525";
const DEFAULT_MOD_ADDRESS: &str = "127.0.0.1:15526";

pub(crate) struct RuntimeService {
    config: RuntimeConfig,
    lease_active: bool,
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
        Ok(Self {
            config,
            lease_active: false,
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
            ("POST", path) if path == self.release_path() && request.body.is_empty() => {
                self.release(request)
            }
            _ => (404, json_error("route_not_found")),
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

fn read_error_status(error: ReadError) -> u16 {
    match error {
        ReadError::Timeout => 504,
        ReadError::Malformed | ReadError::Oversized => 502,
        ReadError::Unavailable => 503,
    }
}
