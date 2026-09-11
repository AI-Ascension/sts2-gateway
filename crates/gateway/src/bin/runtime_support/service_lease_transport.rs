// SPDX-License-Identifier: MIT

//! Fixed loopback transport for legacy runtime relay requests.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::super::http::{
    HttpResponse, MAX_RESPONSE_BYTES, read_response_with_limit, write_request,
};
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
        self.forward_mod_with_limit(method, path, body, correlation, MAX_RESPONSE_BYTES)
    }

    pub(super) fn forward_mod_with_limit(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        correlation: Option<&str>,
        max_response_bytes: usize,
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
            // A recovered lease supersedes the process configuration.  The inbound
            // fence is checked against this lease, so forwarding the configured
            // pre-restart identity would let the mod observe a different fence
            // from the one admitted by the gateway.
            let (instance_id, lease_id, lease_epoch) = self.recovery_lease.as_ref().map_or_else(
                || {
                    (
                        self.config.instance_id.as_str(),
                        self.config.lease_id.as_str(),
                        self.config.lease_epoch,
                    )
                },
                |lease| {
                    (
                        lease.instance_id.as_str(),
                        lease.lease_id.as_str(),
                        lease.lease_epoch,
                    )
                },
            );
            headers.insert(String::from("x-sts2-instance-id"), instance_id.to_owned());
            headers.insert(
                String::from("x-sts2-caller-id"),
                self.config.caller_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-session-id"),
                self.config.session_id.clone(),
            );
            headers.insert(String::from("x-sts2-lease-id"), lease_id.to_owned());
            headers.insert(String::from("x-sts2-lease-epoch"), lease_epoch.to_string());
            headers.insert(
                String::from("x-sts2-correlation-id"),
                correlation.to_owned(),
            );
        }
        write_request(&mut stream, method, path, &headers, body, expires).map_err(|_| 503_u16)?;
        read_response_with_limit(&mut stream, expires, max_response_bytes)
            .map_err(read_error_status)
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};
    use sts2_gateway::RecoveryLease;

    use super::super::super::http::{read_request, write_response};
    use super::super::test_support::test_service;

    fn recovered_lease() -> RecoveryLease {
        RecoveryLease {
            deployment_id: String::from("deployment-1"),
            instance_id: String::from("recovered-instance-2"),
            instance_incarnation: String::from("incarnation-1"),
            boot_id: String::from("boot-2"),
            authority_generation: 2,
            lease_id: String::from("recovered-lease-2"),
            lease_epoch: 2,
            fence_token: String::from("A").repeat(43),
            issued_at_millis: 1,
            expires_at_millis: 2,
            ttl_seconds: 30,
            renewal_interval_seconds: 10,
            last_renew_sequence: 0,
        }
    }

    #[test]
    fn forwarding_uses_the_recovered_lease_fence() -> Result<(), String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            let request = read_request(&mut stream).map_err(|error| error.to_string())?;
            if request
                .headers
                .get("x-sts2-instance-id")
                .map(String::as_str)
                != Some("recovered-instance-2")
                || request.headers.get("x-sts2-lease-id").map(String::as_str)
                    != Some("recovered-lease-2")
                || request
                    .headers
                    .get("x-sts2-lease-epoch")
                    .map(String::as_str)
                    != Some("2")
            {
                return Err(String::from(
                    "forwarded configured rather than recovered lease",
                ));
            }
            write_response(&mut stream, 200, b"{}").map_err(|error| error.to_string())
        });

        let mut service = test_service()?;
        service.config.mod_address = address.to_string();
        service.recovery_lease = Some(recovered_lease());
        assert_eq!(
            service
                .forward_mod("GET", "/api/test", &[], Some("correlation-1"))
                .map_err(|status| format!("forward status {status}"))?
                .status,
            200
        );
        worker
            .join()
            .map_err(|_| String::from("forwarding worker panicked"))??;
        Ok(())
    }
}
