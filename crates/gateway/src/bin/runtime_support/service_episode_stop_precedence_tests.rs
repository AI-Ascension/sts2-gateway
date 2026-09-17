// SPDX-License-Identifier: MIT

//! Stop precedence over the repeated-episode profile.
//!
//! A profiled release may reopen admission only for a *live* episode. If a stop
//! is already in force when the release arrives, the profiled path must not
//! clear it: that is the difference between *completing* an episode and
//! *overriding* a stop. The reachable degraded case is a stop whose host
//! acknowledgment was lost, because the durable binding is then
//! `PendingHostRevoke`, the local lease is retained for idempotent retry, and
//! the permanent flag is already set. `pending_host_revoke_matches` lets that
//! retry past `check_lease`, so the reopen decision must not be read from the
//! post-revoke flag.
//!
//! Real-process synthetic evidence: the real route boundary over the durable
//! recovery store and signed host lease frames on TCP loopback.

use super::*;

/// Force the operator revoke toward a dead listener so durable revocation
/// commits but no host acknowledgment can arrive.
fn revoke_without_ack(service: &mut RuntimeService, lease: &RecoveryLease) -> Result<(), String> {
    service.config.mod_address = String::from("127.0.0.1:9");
    let failed = service.revoke_host_lease(&lease.proof(), "operator", &Uuid::new_v4().to_string());
    assert!(
        failed.is_err(),
        "the unreachable host must not acknowledge the operator revoke"
    );
    assert!(
        service.lease_revoked,
        "a failed operator revoke must leave admission closed"
    );
    assert_eq!(
        binding(service, &lease.lease_id)?,
        RecoveryHostLeaseState::PendingHostRevoke,
        "the durable lease must await the lost operator acknowledgement"
    );
    Ok(())
}

fn assert_stop_still_dominates(service: &mut RuntimeService) -> Result<(), String> {
    let (status, allocation) = service.handle_request(&allocate_request(service));
    assert_eq!(
        status, 409,
        "a stopped episode must not admit a fresh allocation: {}",
        String::from_utf8_lossy(&allocation)
    );
    assert_eq!(error_code(&allocation)?, "lease_context_revoked");
    Ok(())
}

#[test]
fn a_profiled_release_cannot_reopen_admission_after_an_operator_stop() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    revoke_without_ack(&mut service, &lease)?;

    // The retrying release carries the profile and reaches the host, so the
    // revoke is acknowledged durably — yet the stop must survive it.
    let (address, revoker) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke],
        None,
    )?;
    service.config.mod_address = address;
    let (status, body) = service.handle_request(&release_request(
        &service,
        &lease,
        Some("repeated-episode-lease-v1"),
    ));
    revoker
        .join()
        .map_err(|_| String::from("revoker panicked"))??;
    let released: Value =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert!(
        service.lease_revoked,
        "a release after an operator stop must not reopen admission: {released}"
    );
    assert!(
        released.get("episode_profile").is_none(),
        "a stop retry must not arm the repeatable-episode capability: {released}"
    );
    assert!(
        service.episode_profile.is_none(),
        "a stop retry must not leave a reopenable profile behind"
    );
    assert_stop_still_dominates(&mut service)?;
    runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

/// A header-less release must stay byte-identical even *after* a completed
/// profiled episode, because the legacy body is a compatibility promise. The
/// stored profile persists to enforce the epoch floor, but it must not be
/// echoed onto a release that did not negotiate one.
#[test]
fn a_headerless_release_after_a_profiled_episode_stays_byte_identical() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let first = begin_episode(&mut service)?;
    let (status, released) =
        end_episode(&mut service, &first, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200, "episode one failed: {released}");
    assert!(service.episode_profile.is_some(), "episode one must arm the profile");

    let second = begin_episode(&mut service)?;
    let (status, legacy) = end_episode(&mut service, &second, None)?;
    assert_eq!(status, 200, "legacy release failed: {legacy}");
    assert!(
        legacy.get("episode_profile").is_none(),
        "a header-less release must not echo the stored profile: {legacy}"
    );
    assert_eq!(
        legacy,
        serde_json::json!({
            "status": "released",
            "instance_id": service.config.instance_id,
            "lease_id": service.config.lease_id,
            "lease_epoch": service.config.lease_epoch,
        }),
        "the legacy release body must stay byte-identical"
    );
    assert!(
        service.lease_revoked,
        "a header-less release must leave admission permanently closed"
    );
    runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn a_profiled_release_cannot_reopen_admission_after_a_cleanup_stop() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    // The cleanup stop leaves a retained retry marker bound to this lease.
    service.config.mod_address = String::from("127.0.0.1:9");
    allocation_cleanup::cleanup_unreturned_allocation(&mut service);
    assert!(service.lease_revoked, "a cleanup stop must close admission");
    assert_eq!(
        binding(&service, &lease.lease_id)?,
        RecoveryHostLeaseState::PendingHostRevoke,
        "the durable lease must await the lost cleanup acknowledgement"
    );

    let (address, revoker) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke],
        None,
    )?;
    service.config.mod_address = address;
    let (status, body) = service.handle_request(&release_request(
        &service,
        &lease,
        Some("repeated-episode-lease-v1"),
    ));
    revoker
        .join()
        .map_err(|_| String::from("revoker panicked"))??;
    let released: Value =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert!(
        service.lease_revoked,
        "a release after a cleanup stop must not reopen admission: {released}"
    );
    assert_stop_still_dominates(&mut service)?;
    runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

/// Positive control: the precedence guard must not refuse a *live* episode.
#[test]
fn a_profiled_release_of_a_live_episode_still_reopens_admission() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    assert!(
        !service.stop_is_already_in_force(),
        "an installed episode must read as live"
    );
    let (status, released) = end_episode(&mut service, &lease, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert!(
        !service.lease_revoked,
        "a completed live episode must reopen admission: {released}"
    );
    runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

/// The strictest reachable case: the profile is already armed by a *prior*
/// completed episode, so the bind check alone cannot distinguish a live episode
/// from a stop. An operator stop that loses its acknowledgment must still
/// dominate a later profiled release of the same boot.
#[test]
fn a_stop_after_a_completed_episode_still_dominates_a_later_profiled_release()
-> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let first = begin_episode(&mut service)?;
    let (status, released) =
        end_episode(&mut service, &first, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200, "episode one failed: {released}");
    assert!(service.episode_profile.is_some(), "episode one must arm the profile");

    // Episode two is live, then an operator stop loses its host ACK.
    let second = begin_episode(&mut service)?;
    assert!(second.lease_epoch > first.lease_epoch);
    revoke_without_ack(&mut service, &second)?;
    assert!(
        service.episode_profile.is_some(),
        "the armed profile must still bind the same boot"
    );

    let (address, revoker) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke],
        None,
    )?;
    service.config.mod_address = address;
    let (status, body) = service.handle_request(&release_request(
        &service,
        &second,
        Some("repeated-episode-lease-v1"),
    ));
    revoker
        .join()
        .map_err(|_| String::from("revoker panicked"))??;
    let released: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert!(
        service.lease_revoked,
        "an armed profile must not let a later release override an operator stop: {released}"
    );
    assert_stop_still_dominates(&mut service)?;
    runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
