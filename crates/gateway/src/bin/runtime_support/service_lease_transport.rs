// SPDX-License-Identifier: MIT

//! Fixed loopback transport for legacy runtime relay requests.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::super::http::{HttpResponse, read_response, write_request};
use super::RuntimeService;
use super::support::read_error_status;

impl RuntimeService {
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
