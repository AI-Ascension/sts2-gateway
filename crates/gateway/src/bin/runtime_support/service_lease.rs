// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn allocate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
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

    pub(super) fn release(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
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
            ("x-mcp-session-id", self.config.mcp_session_id.as_str()),
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

    pub(super) fn forward_mod(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        correlation: Option<&str>,
    ) -> Result<HttpResponse, u16> {
        let expires = Instant::now() + Duration::from_secs(5);
        let address = self
            .config
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| 503_u16)?;
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
        write_request(&mut stream, method, path, &headers, body, expires).map_err(|_| 503_u16)?;
        read_response(&mut stream, expires).map_err(read_error_status)
    }
}
