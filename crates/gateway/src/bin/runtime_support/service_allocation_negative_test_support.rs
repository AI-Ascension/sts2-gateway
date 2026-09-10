// SPDX-License-Identifier: MIT

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use serde_json::{Value, json};
use sts2_gateway::{RecoveryLease};
use uuid::Uuid;

use super::super::super::host_lease_control::HostLeaseKind;
use super::super::super::recovery_frame::{RecoveryKind, request_frame};
use super::super::test_support::authenticated_request;
use super::super::*;

const TEST_CALLER: &str = "00000000-0000-4000-8000-000000000008";
const TEST_SESSION: &str = "00000000-0000-4000-8000-000000000007";

/// A bounded signed host fake.  The optional final request lets the same test
/// observe both the unfixed path (no cleanup/fresh install) and an eventual
/// repaired path without making the unfixed run hang for two seconds.
pub(super) fn spawn_signed_ack_server(
    key: Vec<u8>,
    principal: String,
    kinds: Vec<HostLeaseKind>,
    optional_last: bool,
    lock: Option<(PathBuf, Duration)>,
) -> Result<(String, thread::JoinHandle<Result<(), String>>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let kind_count = kinds.len();
        for (index, kind) in kinds.into_iter().enumerate() {
            let timeout = if optional_last && index + 1 == kind_count {
                Duration::from_millis(300)
            } else {
                Duration::from_secs(2)
            };
            let Some(mut stream) = accept_with_timeout(&listener, timeout)? else {
                return Ok(());
            };
            let request = super::super::super::http::read_request(&mut stream)
                .map_err(|status| format!("host request read failed with {status}"))?;
            let request = super::super::super::host_lease_control::parse_request(&request.body, kind)
                .map_err(|error| format!("host request parse failed: {error:?}"))?;
            let payload = request["payload"].clone();
            let grant = payload["grant"].clone();

            let mut blocker = None;
            if index == 0
                && let Some((path, hold_for)) = lock.as_ref()
            {
                let connection = Connection::open(path).map_err(|error| {
                    format!("host fake could not open allocation store for lock: {error}")
                })?;
                connection
                    .execute_batch("BEGIN IMMEDIATE")
                    .map_err(|error| format!("host fake could not hold allocation lock: {error}"))?;
                blocker = Some((connection, *hold_for));
            }

            let mut response = json!({
                "contract": sts2_gateway::HOST_LEASE_CONTROL_CONTRACT,
                "schema_digest": sts2_gateway::HOST_LEASE_CONTROL_SCHEMA_DIGEST,
                "message_id": Uuid::new_v4().to_string(),
                "correlation_id": request["correlation_id"].clone(),
                "sent_at": request["sent_at"].clone(),
                "actor": {"principal_id": principal.clone(), "role": "host"},
                "auth": {
                    "principal_id": principal.clone(),
                    "capability": kind.capability(),
                    "proof": "placeholder"
                },
                "kind": kind.response_name(),
                "payload": {
                    "ack": {
                        "result": {
                            "status": if kind == HostLeaseKind::Revoke { "REVOKED" } else { "INSTALLED" },
                            "retryable": false,
                            "retry_after_seconds": null
                        },
                        "installation_id": payload["installation_id"].clone(),
                        "grant_digest": payload["grant_digest"].clone(),
                        "boot_id": grant["boot"]["boot_id"].clone(),
                        "instance_incarnation": grant["boot"]["instance_incarnation"].clone(),
                        "host_fence_id": grant["fence"]["host_fence_id"].clone(),
                        "fence_generation": grant["fence"]["fence_generation"].clone(),
                        "lease_id": grant["lease"]["lease_id"].clone(),
                        "lease_epoch": grant["lease"]["lease_epoch"].clone(),
                        "host_install_generation": 1,
                        "recorded_at": request["sent_at"].clone(),
                        "renew_sequence": null,
                        "expires_at": if kind == HostLeaseKind::Revoke {
                            Value::Null
                        } else {
                            grant["lease"]["expires_at"].clone()
                        }
                    }
                }
            });
            let proof = super::super::super::host_lease_control_crypto::proof_for_frame(
                &response,
                kind.acknowledgment_domain(),
                &key,
            );
            response["auth"]["proof"] = Value::String(proof);
            let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
            super::super::super::host_lease_control::parse_response(
                &body,
                kind,
                &key,
                response["actor"]["principal_id"]
                    .as_str()
                    .ok_or_else(|| String::from("response principal missing"))?,
            )
            .map_err(|error| format!("constructed host response rejected: {error:?}"))?;
            super::super::super::http::write_response(&mut stream, 200, &body)
                .map_err(|error| error.to_string())?;

            if let Some((connection, hold_for)) = blocker {
                thread::sleep(hold_for);
                connection
                    .execute_batch("ROLLBACK")
                    .map_err(|error| format!("host fake could not release allocation lock: {error}"))?;
            }
        }
        Ok(())
    });
    Ok((address.to_string(), server))
}

pub(super) fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> Result<Option<TcpStream>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(Some(stream)),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(format!("host request accept failed: {error}")),
        }
    }
}

pub(super) fn retire_fixture_lease(
    service: &mut RuntimeService,
    old_lease: &RecoveryLease,
) -> Result<(), String> {
    service.config.caller_id = TEST_CALLER.to_owned();
    service.config.session_id = TEST_SESSION.to_owned();
    service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?
        .revoke_lease(&old_lease.proof(), "test_cleanup")
        .map_err(|error| error.to_string())?;
    service.recovery_lease = None;
    service.recovery_lease_deadline = None;
    service.recovery_lease_deadline_lease_id = None;
    service.recovery_host_grant = None;
    service.lease_active = false;
    service.lease_revoked = false;
    Ok(())
}

pub(super) fn allocation_body(service: &RuntimeService) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({
        "instance_id": service.config.instance_id,
        "caller_id": service.config.caller_id,
        "session_id": service.config.session_id,
    }))
    .map_err(|error| error.to_string())
}

pub(super) fn recovery_revoke_request(
    service: &RuntimeService,
    lease: &RecoveryLease,
) -> HttpRequest {
    let correlation = Uuid::new_v4().to_string();
    let mut request = authenticated_request("/v1/recovery/lease/revoke");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.headers.insert(
        String::from("x-sts2-instance-id"),
        service.config.instance_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-caller-id"),
        service.config.caller_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-session-id"),
        service.config.session_id.clone(),
    );
    request
        .headers
        .insert(String::from("x-sts2-lease-id"), lease.lease_id.clone());
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), correlation.clone());
    request.headers.insert(
        String::from("x-sts2-recovery-capability"),
        String::from("lease_revoke"),
    );
    request.body = request_frame(
        RecoveryKind::LeaseRevoke,
        &service.config.caller_id,
        &correlation,
        None,
        json!({
            "lease": super::super::recovery_wire::lease_value(lease),
            "reason": "shutdown",
        }),
    );
    request
}

pub(super) fn runtime_request_for(
    service: &RuntimeService,
    lease: &RecoveryLease,
) -> HttpRequest {
    let mut request = authenticated_request("/v1/instances/unused/action");
    request.headers.insert(
        String::from("x-sts2-instance-id"),
        lease.instance_id.clone(),
    );
    request
        .headers
        .insert(String::from("x-sts2-caller-id"), service.config.caller_id.clone());
    request
        .headers
        .insert(String::from("x-sts2-session-id"), service.config.session_id.clone());
    request
        .headers
        .insert(String::from("x-sts2-lease-id"), lease.lease_id.clone());
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    request
}
