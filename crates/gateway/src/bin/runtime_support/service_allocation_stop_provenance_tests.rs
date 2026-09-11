// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Copy, Debug)]
enum Stop {
    AlreadyRevoked,
    Operator,
    Release,
    Shutdown,
    ShutdownRequested,
    OtherLeaseMarker,
    ExternalShutdown,
}

#[test]
fn delayed_cleanup_ack_never_overrides_stop_or_other_lease_provenance() -> Result<(), String> {
    for stop in [
        Stop::AlreadyRevoked,
        Stop::Operator,
        Stop::Release,
        Stop::Shutdown,
        Stop::ShutdownRequested,
        Stop::OtherLeaseMarker,
        Stop::ExternalShutdown,
    ] {
        verify_stop(stop)?;
    }
    Ok(())
}

fn verify_stop(stop: Stop) -> Result<(), String> {
    let (mut service, old_lease, path) =
        super::super::runtime_v3_catalog_tests::recovery_service()?;
    retire_fixture_lease(&mut service, &old_lease)?;
    let allocation = allocation_body(&service)?;
    let (address, installer) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install],
        false,
        None,
    )?;
    service.config.mod_address = address;
    assert_eq!(service.allocate(&allocation).0, 200, "{stop:?}");
    installer.join().map_err(|_| String::from("installer panicked"))??;
    let lease = service.recovery_lease.clone()
        .ok_or_else(|| String::from("installed lease missing"))?;
    if matches!(stop, Stop::AlreadyRevoked) {
        service.lease_revoked = true;
    }
    // The listener is gone: durable revocation succeeds but no host ACK is available.
    super::super::allocation_cleanup::cleanup_unreturned_allocation(&mut service);
    assert!(!service.lease_active);
    assert!(service.lease_revoked);
    assert!(service.check_lease(&runtime_request_for(&service, &lease)).is_err());
    let binding = service.recovery.as_ref()
        .ok_or_else(|| String::from("store missing"))?
        .host_lease_binding(&lease.lease_id).map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::PendingHostRevoke, "{stop:?}");
    if matches!(stop, Stop::ShutdownRequested) {
        service.shutdown_requested = true;
    }
    if matches!(stop, Stop::OtherLeaseMarker) {
        service.allocation_cleanup_lease_id = Some(Uuid::new_v4().to_string());
    }
    let (address, revoker) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke],
        false,
        None,
    )?;
    service.config.mod_address = address;
    let request = recovery_revoke_request(&service, &lease);
    match stop {
        Stop::Operator => service.revoke_host_lease(
            &lease.proof(), "operator", &Uuid::new_v4().to_string(),
        ).map_err(|error| format!("operator revoke failed: {error:?}"))?,
        Stop::Release => assert_eq!(service.release(&request).0, 200),
        Stop::Shutdown => assert_eq!(service.runtime_v2_shutdown(&request).0, 202),
        _ => assert_eq!(service.handle_request(&request).0, 200),
    }
    revoker.join().map_err(|_| String::from("revoker panicked"))??;
    assert!(service.lease_revoked, "ACK reopened stopped admission: {stop:?}");
    assert!(!service.lease_active);
    assert!(service.recovery_lease.is_none());
    assert!(service.allocation_cleanup_lease_id.is_none());
    assert_ne!(service.allocate(&allocation).0, 200, "{stop:?}");
    let binding = service.recovery.as_ref()
        .ok_or_else(|| String::from("store missing"))?
        .host_lease_binding(&lease.lease_id).map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("binding missing"))?;
    assert_eq!(binding.state, RecoveryHostLeaseState::HostRevoked, "{stop:?}");
    super::super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
