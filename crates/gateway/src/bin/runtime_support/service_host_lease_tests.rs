// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{
    GatewayRecoveryStore, HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST,
    RecoveryHostLeaseState, RecoveryLease, RecoveryLeaseRequest,
};
use uuid::Uuid;

use super::super::test_support::test_service;

fn lease() -> RecoveryLease {
    RecoveryLease {
        deployment_id: String::from("00000000-0000-4000-8000-000000000001"),
        instance_id: String::from("00000000-0000-4000-8000-000000000002"),
        instance_incarnation: String::from("00000000-0000-4000-8000-000000000003"),
        boot_id: String::from("00000000-0000-4000-8000-000000000004"),
        authority_generation: 1,
        lease_id: String::from("00000000-0000-4000-8000-000000000006"),
        lease_epoch: 1,
        fence_token: String::from("A").repeat(43),
        issued_at_millis: 1_000,
        expires_at_millis: 10_000,
        ttl_seconds: 30,
        renewal_interval_seconds: 10,
        last_renew_sequence: 0,
    }
}

#[test]
fn duplicate_install_does_not_refresh_deadline_after_wall_clock_regression() {
    let mut service = test_service().expect("test service");
    let current_lease = lease();
    let original_deadline = Instant::now() + Duration::from_secs(2);
    service.recovery_lease = Some(current_lease.clone());
    service.recovery_lease_deadline = Some(original_deadline);
    service.recovery_lease_deadline_lease_id = Some(current_lease.lease_id.clone());

    service
        .establish_install_deadline(&current_lease, Instant::now(), 9_000)
        .expect("existing deadline is still live");
    service
        .establish_install_deadline(&current_lease, Instant::now(), 8_000)
        .expect("wall-clock regression does not expire the lease");

    assert_eq!(service.recovery_lease_deadline, Some(original_deadline));
}

#[test]
fn expired_install_deadline_cannot_be_recreated_for_the_same_lease() {
    let mut service = test_service().expect("test service");
    let current_lease = lease();
    service.recovery_lease = Some(current_lease.clone());
    service.recovery_lease_deadline = Some(Instant::now() - Duration::from_millis(1));
    service.recovery_lease_deadline_lease_id = Some(current_lease.lease_id.clone());

    service.check_recovery_deadline();
    assert!(service.recovery_lease.is_none());
    assert!(!service.lease_active);
    for _ in 0..3 {
        assert_eq!(
            service.establish_install_deadline(&current_lease, Instant::now(), 9_000),
            Err(super::HostLeaseFailure::expired())
        );
    }
    assert_eq!(
        service.recovery_lease_deadline_lease_id,
        Some(current_lease.lease_id)
    );
}

#[test]
fn installed_same_lease_id_cannot_reenter_after_expiry() {
    let mut service = test_service().expect("test service");
    let current_lease = lease();
    service.recovery_lease = Some(current_lease.clone());
    service.recovery_lease_deadline_lease_id = Some(current_lease.lease_id.clone());

    assert_eq!(
        service.establish_install_deadline(&current_lease, Instant::now(), 9_000),
        Err(super::HostLeaseFailure::expired())
    );
}

#[test]
fn a_different_fresh_lease_identity_may_establish_a_deadline() {
    let mut service = test_service().expect("test service");
    let previous_lease = lease();
    let mut fresh_lease = lease();
    fresh_lease.lease_id = String::from("00000000-0000-4000-8000-000000000007");
    service.recovery_lease_deadline_lease_id = Some(previous_lease.lease_id);

    service
        .establish_install_deadline(&fresh_lease, Instant::now(), 9_000)
        .expect("fresh lease identity gets one deadline");
    assert_eq!(
        service.recovery_lease_deadline_lease_id,
        Some(fresh_lease.lease_id)
    );
    assert!(service.recovery_lease_deadline.is_some());
}

#[test]
fn delayed_revoke_ack_after_expiry_commits_host_revoked_without_active_lease() -> Result<(), String>
{
    let mut service = test_service()?;
    const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
    const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
    const CORRELATION: &str = "00000000-0000-4000-8000-000000000012";
    service.config.instance_id = INSTANCE.to_owned();
    service.config.caller_id = String::from("00000000-0000-4000-8000-000000000008");
    service.config.session_id = String::from("00000000-0000-4000-8000-000000000007");

    let path = std::env::temp_dir().join(format!(
        "sts2-gateway-host-revoke-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let now = service.recovery_now_millis();
    let boot = store
        .start_boot(
            DEPLOYMENT,
            INSTANCE,
            service.config.recovery_release.clone(),
            now,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, now.saturating_add(1))
        .map_err(|error| error.to_string())?;
    let lease = store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id: DEPLOYMENT.to_owned(),
            instance_id: INSTANCE.to_owned(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id: service.config.caller_id.clone(),
            session_id: service.config.session_id.clone(),
            now_millis: now.saturating_add(2),
            ttl_seconds: service.config.recovery_ttl_seconds,
            renewal_interval_seconds: service.config.recovery_renewal_interval_seconds,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant = super::super::host_lease_helpers::grant_value(
        &boot,
        &fence,
        &lease,
        &service.config.caller_id,
        &service.config.session_id,
    );
    let grant_digest = super::super::super::host_lease_control::grant_digest(&grant)
        .map_err(|error| format!("grant digest: {error:?}"))?;
    store
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            &fence.host_fence_id,
            fence.fence_generation,
            now.saturating_add(3),
        )
        .map_err(|error| format!("prepare install: {error}"))?;
    store
        .complete_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            1,
            &Uuid::new_v4().to_string(),
            now.saturating_add(4),
        )
        .map_err(|error| format!("complete install: {error}"))?;

    let expiry = lease.expires_at_millis;
    let key = service.config.host_lease_key.clone();
    let principal = service.config.host_principal_id.clone();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(format!("host request accept failed: {error}")),
            }
        };
        let request = super::super::super::http::read_request(&mut stream)
            .map_err(|status| format!("host request read failed with {status}"))?;
        let request = super::super::super::host_lease_control::parse_request(
            &request.body,
            super::super::super::host_lease_control::HostLeaseKind::Revoke,
        )
        .map_err(|error| format!("host request parse failed: {error:?}"))?;
        let payload = request["payload"].clone();
        let grant = payload["grant"].clone();
        let mut response = json!({
            "contract": HOST_LEASE_CONTROL_CONTRACT,
            "schema_digest": HOST_LEASE_CONTROL_SCHEMA_DIGEST,
            "message_id": Uuid::new_v4().to_string(),
            "correlation_id": request["correlation_id"].clone(),
            "sent_at": super::super::super::recovery_frame::timestamp_from_millis(
                expiry.saturating_add(1),
            ),
            "actor": {"principal_id": principal, "role": "host"},
            "auth": {
                "principal_id": principal,
                "capability": "lease_revoke",
                "proof": "placeholder"
            },
            "kind": "lease_revoke_response",
            "payload": {
                "ack": {
                    "result": {"status": "REVOKED", "retryable": false, "retry_after_seconds": null},
                    "installation_id": payload["installation_id"].clone(),
                    "grant_digest": payload["grant_digest"].clone(),
                    "boot_id": grant["boot"]["boot_id"].clone(),
                    "instance_incarnation": grant["boot"]["instance_incarnation"].clone(),
                    "host_fence_id": grant["fence"]["host_fence_id"].clone(),
                    "fence_generation": grant["fence"]["fence_generation"].clone(),
                    "lease_id": grant["lease"]["lease_id"].clone(),
                    "lease_epoch": grant["lease"]["lease_epoch"].clone(),
                    "host_install_generation": 1,
                    "recorded_at": super::super::super::recovery_frame::timestamp_from_millis(
                        expiry.saturating_add(1),
                    ),
                    "renew_sequence": null,
                    "expires_at": null
                }
            }
        });
        let proof = super::super::super::host_lease_control_crypto::proof_for_frame(
            &response,
            super::super::super::host_lease_control::HostLeaseKind::Revoke.acknowledgment_domain(),
            &key,
        );
        response["auth"]["proof"] = Value::String(proof);
        let body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        super::super::super::host_lease_control::parse_response(
            &body,
            super::super::super::host_lease_control::HostLeaseKind::Revoke,
            &key,
            response["actor"]["principal_id"]
                .as_str()
                .ok_or_else(|| String::from("response principal missing"))?,
        )
        .map_err(|error| format!("constructed host response rejected: {error:?}"))?;
        super::super::super::http::write_response(&mut stream, 200, &body)
            .map_err(|error| error.to_string())
    });

    service.config.mod_address = address.to_string();
    service.recovery = Some(store);
    service.recovery_boot = Some(boot);
    service.recovery_fence = Some(fence);
    service.recovery_lease_deadline = Some(Instant::now() + Duration::from_secs(30));
    service.recovery_lease_deadline_lease_id = Some(lease.lease_id.clone());
    service.recovery_lease = Some(lease.clone());
    service.lease_active = true;

    let revoke = service.revoke_host_lease(&lease.proof(), "shutdown", CORRELATION);
    let server_result = server
        .join()
        .map_err(|_| String::from("host response server panicked"))?;
    if let Err(error) = revoke {
        return Err(format!(
            "revoke failed: {error:?}; host server: {server_result:?}"
        ));
    }
    server_result?;

    assert!(!service.lease_active);
    assert!(service.recovery_lease.is_none());
    let binding = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&lease.lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host lease binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::HostRevoked);
    assert_eq!(
        binding.host_ack_recorded_at_millis,
        Some(expiry.saturating_add(1))
    );

    drop(service);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn recovery_clock_never_moves_backwards() {
    let mut service = test_service().expect("test service");
    service.recovery_clock_wall_millis = 10_000;
    service.recovery_last_now_millis = 20_000;

    assert!(service.recovery_now_millis() >= 20_000);
}
