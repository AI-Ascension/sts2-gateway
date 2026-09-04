// SPDX-License-Identifier: MIT

use super::{HttpRequest, MAX_BODY_BYTES, RuntimeService, json_bytes, json_error, safe_identity};
use serde_json::{Value, json};
use std::collections::BTreeMap;

impl RuntimeService {
    pub(super) fn health(&self) -> (u16, Vec<u8>) {
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

    pub(super) fn allocate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        if self.lease_released {
            return (409, json_error("lease_context_revoked"));
        }
        let Ok(value) = super::super::strict_json::parse(body) else {
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

    pub(super) fn release(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.lease_active = false;
        self.lease_released = true;
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

    pub(super) fn relay_data(
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

    pub(super) fn check_lease(&self, request: &HttpRequest) -> Result<(), (u16, Vec<u8>)> {
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

    pub(super) fn has_gateway_token(&self, request: &HttpRequest) -> bool {
        let expected = format!("Bearer {}", self.config.gateway_token);
        request.headers.get("authorization").map(String::as_str) == Some(expected.as_str())
    }

    pub(super) fn state_path(&self) -> String {
        format!("/v1/instances/{}/state", self.config.instance_id)
    }

    pub(super) fn action_path(&self) -> String {
        format!("/v1/instances/{}/action", self.config.instance_id)
    }

    pub(super) fn release_path(&self) -> String {
        format!("/v1/instances/{}/release", self.config.instance_id)
    }
}
pub(super) fn headers_are_allowed(headers: &BTreeMap<String, String>) -> bool {
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
