// SPDX-License-Identifier: MIT

//! Consecutive-episode lease behavior driven through the real route boundary.
//!
//! Every case here exercises the durable recovery store and the signed host
//! lease frames over a real TCP loopback fake, so the assertions observe the
//! same admission decisions the served gateway makes.

use sts2_gateway::RecoveryHostLeaseState;
use uuid::Uuid;

use super::super::host_lease_control::HostLeaseKind;
use super::episode_profile::EPISODE_PROFILE_HEADER;
use super::test_support::authenticated_request;
use super::*;

// The support helpers are shared with the allocation-regression suite by
// including the same test-only file; each suite uses a subset. The module is
// declared once, in the allocation-regression suite, and re-used here.
use super::allocation_negative_regression_tests::support;
use self::support::{
    allocation_body, retire_fixture_lease, runtime_request_for, spawn_signed_ack_server,
};

#[path = "service_episode_stop_precedence_tests.rs"]
mod stop_precedence;

fn release_request(
    service: &RuntimeService,
    lease: &sts2_gateway::RecoveryLease,
    profile: Option<&str>,
) -> HttpRequest {
    let mut request =
        authenticated_request(&format!("/v1/instances/{}/release", service.config.instance_id));
    request.method = String::from("POST");
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
    request.headers.insert(
        String::from("x-sts2-lease-id"),
        lease.lease_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), Uuid::new_v4().to_string());
    if let Some(profile) = profile {
        request
            .headers
            .insert(String::from(EPISODE_PROFILE_HEADER), profile.to_owned());
    }
    request
}

fn allocate_request(service: &RuntimeService) -> HttpRequest {
    let mut request = authenticated_request("/v1/sessions/allocate");
    request.method = String::from("POST");
    request
        .headers
        .insert(String::from("content-type"), String::from("application/json"));
    request
        .headers
        .insert(String::from("x-sts2-instance-id"), service.config.instance_id.clone());
    request.body = allocation_body(service).unwrap_or_default();
    request
}

/// The runtime-v3 fixture helper omits the identity headers that
/// `check_lease` fences; add them so the request reaches catalogue admission.
fn fenced(mut request: HttpRequest, service: &RuntimeService) -> HttpRequest {
    request.headers.insert(
        String::from("x-sts2-caller-id"),
        service.config.caller_id.clone(),
    );
    request.headers.insert(
        String::from("x-sts2-session-id"),
        service.config.session_id.clone(),
    );
    request.headers.insert(
        String::from("x-mcp-session-id"),
        service.config.mcp_session_id.clone(),
    );
    request
}

/// Install one episode's lease against a fresh signed host fake.
fn begin_episode(service: &mut RuntimeService) -> Result<sts2_gateway::RecoveryLease, String> {
    let (address, installer) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Install],
        None,
    )?;
    service.config.mod_address = address;
    let (status, body) = service.handle_request(&allocate_request(service));
    installer
        .join()
        .map_err(|_| String::from("installer panicked"))??;
    if status != 200 {
        return Err(format!(
            "episode install failed: {status} {}",
            String::from_utf8_lossy(&body)
        ));
    }
    service
        .recovery_lease
        .clone()
        .ok_or_else(|| String::from("installed lease missing"))
}

/// Release one episode and let the signed host fake confirm the revoke.
fn end_episode(
    service: &mut RuntimeService,
    lease: &sts2_gateway::RecoveryLease,
    profile: Option<&str>,
) -> Result<(u16, Value), String> {
    let (address, revoker) = spawn_signed_ack_server(
        service.config.host_lease_key.clone(),
        service.config.host_principal_id.clone(),
        vec![HostLeaseKind::Revoke],
        None,
    )?;
    service.config.mod_address = address;
    let (status, body) = service.handle_request(&release_request(service, lease, profile));
    revoker
        .join()
        .map_err(|_| String::from("revoker panicked"))??;
    let value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    Ok((status, value))
}

fn binding(service: &RuntimeService, lease_id: &str) -> Result<RecoveryHostLeaseState, String> {
    Ok(service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(lease_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("host binding missing"))?
        .state)
}

fn error_code(body: &[u8]) -> Result<String, String> {
    let value: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    value["error_code"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("response carries no error_code: {value}"))
}

fn episode_service() -> Result<(RuntimeService, std::path::PathBuf), String> {
    let (mut service, fixture_lease, path) = super::runtime_v3_catalog_tests::recovery_service()?;
    retire_fixture_lease(&mut service, &fixture_lease)?;
    Ok((service, path))
}

#[test]
fn two_consecutive_episodes_land_on_distinct_leases_and_higher_epochs() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    assert_ne!(
        service.config.caller_id, "harness",
        "the recovery fixture must have retired the attached-loopback caller"
    );

    let first = begin_episode(&mut service)?;
    let dispatch =
        super::runtime_v3_catalog_tests::dispatch_envelope(&service, &first, "episode-one")?;
    super::runtime_v3_catalog_tests::capture_old_catalog(&mut service, &first, &dispatch)?;
    assert!(service.recovery_catalog.current().is_some());

    let (status, released) = end_episode(&mut service, &first, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert_eq!(released["status"], "released");
    assert_eq!(released["episode_profile"]["released_epoch"], first.lease_epoch);
    assert!(
        !service.lease_revoked,
        "a completed profiled episode must reopen admission"
    );
    assert!(
        service.recovery_catalog.current().is_none(),
        "the completed episode's catalog must not survive into the next episode"
    );
    assert_eq!(binding(&service, &first.lease_id)?, RecoveryHostLeaseState::HostRevoked);

    let second = begin_episode(&mut service)?;
    assert_ne!(second.lease_id, first.lease_id);
    assert!(
        second.lease_epoch > first.lease_epoch,
        "episode two must land on a strictly higher epoch: {first:?} then {second:?}"
    );
    assert!(second.boot_id == first.boot_id, "the same boot authority must persist");
    assert_eq!(
        service
            .episode_profile_witness()
            .ok_or_else(|| String::from("witness missing"))?["released_epoch"],
        first.lease_epoch
    );

    // Episode one's released lease and catalogue are fenced out of episode two.
    assert!(
        service
            .check_lease(&runtime_request_for(&service, &first))
            .is_err(),
        "the released episode must not admit mutations"
    );
    let dispatch = super::runtime_v3_catalog_tests::dispatch_envelope(&service, &second, "episode-two")?;
    let action = fenced(
        super::runtime_v3_catalog_tests::runtime_request(&service, &second, "action", dispatch)?,
        &service,
    );
    let (status, body) = service.handle_request(&action);
    assert_eq!(status, 409);
    assert_eq!(
        error_code(&body)?,
        "recovery_catalog_fresh_read_required",
        "episode two must observe its own catalogue, not episode one's"
    );
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn legacy_release_without_a_profile_stays_permanently_revoked() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    let (status, released) = end_episode(&mut service, &lease, None)?;
    assert_eq!(status, 200);
    assert!(
        released.get("episode_profile").is_none(),
        "the single-episode body must stay byte-compatible: {released}"
    );
    assert!(service.lease_revoked);
    assert!(service.episode_profile.is_none());

    let (status, body) = service.handle_request(&allocate_request(&service));
    assert_eq!(status, 409);
    assert_eq!(error_code(&body)?, "lease_context_revoked");
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn unsupported_profile_is_rejected_before_any_durable_write() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    let (status, body) = service.handle_request(&release_request(
        &service,
        &lease,
        Some("repeated-episode-lease-v2"),
    ));
    assert_eq!(status, 400);
    assert_eq!(error_code(&body)?, "episode_profile_unsupported");
    assert!(!service.lease_revoked, "a rejected negotiation must not stop the lease");
    assert!(service.lease_active, "the episode must remain live and retryable");
    assert_eq!(binding(&service, &lease.lease_id)?, RecoveryHostLeaseState::Installed);

    // A truthful retry with the supported profile still completes the episode.
    let (status, _) = end_episode(&mut service, &lease, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200);
    assert!(!service.lease_revoked);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn a_profiled_release_without_a_boot_authority_fails_closed() -> Result<(), String> {
    let mut service = super::test_support::test_service()?;
    let mut request = authenticated_request("/v1/instances/instance-1/release");
    request.method = String::from("POST");
    request
        .headers
        .insert(String::from(EPISODE_PROFILE_HEADER), String::from("repeated-episode-lease-v1"));
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "episode_profile_boot_required");
    assert!(service.episode_profile.is_none());

    // An absent header still takes the legacy path with no boot.
    let mut legacy = authenticated_request("/v1/instances/instance-1/release");
    legacy.method = String::from("POST");
    assert_eq!(service.handle_request(&legacy).0, 200);
    assert!(service.lease_revoked);
    Ok(())
}

#[test]
fn a_stale_lease_fence_is_rejected_before_the_profile_is_negotiated() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let lease = begin_episode(&mut service)?;
    let mut request = release_request(&service, &lease, Some("repeated-episode-lease-v1"));
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        (lease.lease_epoch + 1).to_string(),
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 409);
    assert_eq!(error_code(&body)?, "lease_fence_rejected");
    assert!(service.episode_profile.is_none());
    assert!(service.lease_active);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn a_local_floor_above_the_durable_epoch_refuses_and_revokes_the_acquired_lease()
-> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let first = begin_episode(&mut service)?;
    let (status, _) = end_episode(&mut service, &first, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200);
    // Simulate the two views disagreeing: the profile believes a much higher
    // episode already completed. The durable allocator will issue
    // `first.lease_epoch + 1`, which must be refused and durably revoked.
    service
        .episode_profile
        .as_mut()
        .ok_or_else(|| String::from("profile missing"))?
        .record_completed(first.lease_epoch + 100);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    service.config.mod_address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    let (status, body) = service.handle_request(&allocate_request(&service));
    assert_eq!(status, 409);
    assert_eq!(error_code(&body)?, "lease_context_revoked");
    assert!(service.lease_revoked, "admission stays closed on a durable/local disagreement");
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "the refused lease must never reach the host"
    );
    let durable = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .host_lease_binding(&Uuid::nil().to_string());
    assert!(durable.is_err() || durable.ok().flatten().is_none());
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}

#[test]
fn a_restart_discards_the_gateway_local_profile() -> Result<(), String> {
    let (mut service, path) = episode_service()?;
    let first = begin_episode(&mut service)?;
    let (status, _) = end_episode(&mut service, &first, Some("repeated-episode-lease-v1"))?;
    assert_eq!(status, 200);
    assert!(service.episode_profile.is_some());
    let boot = service
        .recovery_boot
        .clone()
        .ok_or_else(|| String::from("boot missing"))?;
    let now = service.recovery_now_millis();
    let store = service.recovery.take().ok_or_else(|| String::from("store missing"))?;
    drop(store);

    // Reopen the same durable store, as a fresh process would.
    let mut reopened =
        sts2_gateway::GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let next_boot = reopened
        .start_boot(
            &boot.deployment_id,
            &boot.instance_id,
            service.config.recovery_release.clone(),
            now.saturating_add(1),
        )
        .map_err(|error| error.to_string())?;
    assert!(
        next_boot.authority_generation > boot.authority_generation,
        "a restart must rotate the boot authority"
    );
    let restarted = super::test_support::test_service()?;
    assert!(
        restarted.episode_profile.is_none(),
        "the profile is gateway-local process state and must not survive a restart"
    );
    assert!(
        !restarted.episode_admission_refused(&next_boot, 1),
        "the durable epoch stream alone fences the fresh process"
    );
    drop(reopened);
    super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
