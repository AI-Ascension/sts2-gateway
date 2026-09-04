// SPDX-License-Identifier: MIT

use super::http::{
    HttpRequest, HttpResponse, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, ReadError, read_request,
    read_response, write_request, write_response,
};
use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;
use super::runtime_v3_gameplay_forwarder::{
    RuntimeV3GameplayForwardError, RuntimeV3GameplayForwarder,
};
use serde_json::{Value, json};
use service_config::RuntimeConfig;
use service_control::headers_are_allowed;
use service_v2::UnconfiguredRuntimeV2Forwarder;
use std::net::{TcpListener, TcpStream};
use sts2_gateway::{
    RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Observation,
};

#[path = "service_config.rs"]
mod service_config;
#[path = "service_control.rs"]
mod service_control;
#[path = "service_downstream.rs"]
mod service_downstream;
#[path = "service_v2.rs"]
mod service_v2;
#[path = "service_v3.rs"]
mod service_v3;

pub(crate) struct RuntimeService {
    config: RuntimeConfig,
    lease_active: bool,
    lease_released: bool,
    runtime_v2: RuntimeV2Ledger<UnconfiguredRuntimeV2Forwarder>,
    runtime_v3: RuntimeV3GameplayForwarder,
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
            lease_released: false,
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

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
