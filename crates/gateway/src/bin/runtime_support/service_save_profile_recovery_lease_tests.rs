// SPDX-License-Identifier: MIT

use super::*;
use std::net::TcpListener;
use std::thread;

#[test]
fn save_profile_discovery_uses_the_current_recovery_lease() -> Result<(), String> {
    let mut ready = ready_service()?;
    let lease = ready
        .service
        .recovery_lease
        .clone()
        .ok_or("missing lease")?;
    assert_ne!(lease.lease_id, ready.service.config.lease_id);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    ready.service.save_profile.forwarding_mut().clone_from(
        &super::super::super::super::save_profile_forwarder::HttpSaveProfileForwarder::new(
            &address.to_string(),
            "mod-token",
        ),
    );
    let response = serde_json::to_vec(&json!({
        "status": "settled",
        "instance_id": ready.service.config.instance_id,
        "caller_id": ready.service.config.caller_id,
        "session_id": ready.service.config.session_id,
        "lease_id": lease.lease_id,
        "lease_epoch": lease.lease_epoch,
        "correlation_id": "current-recovery-profile",
        "operation_id": "op-current",
        "profiles": [],
    }))
    .map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<HttpRequest, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let request = super::super::super::super::http::read_request(&mut stream)
            .map_err(|status| format!("read failed: {status}"))?;
        super::super::super::super::http::write_response(&mut stream, 200, &response)
            .map_err(|error| error.to_string())?;
        Ok(request)
    });
    let request = HttpRequest {
        method: "GET".to_owned(),
        path: format!(
            "/v1/instances/{}/save-profile/current",
            ready.service.config.instance_id
        ),
        headers: std::collections::BTreeMap::from([
            (
                "authorization".to_owned(),
                "Bearer recovery-token".to_owned(),
            ),
            (
                "x-sts2-instance-id".to_owned(),
                ready.service.config.instance_id.clone(),
            ),
            (
                "x-sts2-caller-id".to_owned(),
                ready.service.config.caller_id.clone(),
            ),
            (
                "x-sts2-session-id".to_owned(),
                ready.service.config.session_id.clone(),
            ),
            (
                "x-mcp-session-id".to_owned(),
                ready.service.config.mcp_session_id.clone(),
            ),
            ("x-sts2-lease-id".to_owned(), lease.lease_id.clone()),
            (
                "x-sts2-lease-epoch".to_owned(),
                lease.lease_epoch.to_string(),
            ),
            (
                "x-sts2-correlation-id".to_owned(),
                "current-recovery-profile".to_owned(),
            ),
            ("x-mcp-request-id".to_owned(), "op-current".to_owned()),
        ]),
        body: Vec::new(),
    };
    let (status, body) = ready.service.handle_request(&request);
    let forwarded = server.join().map_err(|_| "recording peer panicked")??;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert_eq!(forwarded.path, "/api/v1/save-profile/current");
    assert_eq!(
        forwarded.headers.get("x-sts2-lease-id"),
        Some(&lease.lease_id)
    );
    assert_eq!(
        forwarded.headers.get("x-sts2-lease-epoch"),
        Some(&lease.lease_epoch.to_string())
    );
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}
