// SPDX-License-Identifier: MIT

fn replace_ready_lease(ready: &mut ReadyService) -> Result<(), String> {
    let old_lease = ready
        .service
        .recovery_lease
        .clone()
        .ok_or_else(|| String::from("ready fixture omitted its lease"))?;
    let boot = ready
        .service
        .recovery_boot
        .clone()
        .ok_or_else(|| String::from("ready fixture omitted current boot"))?;
    let fence = ready
        .service
        .recovery_fence
        .clone()
        .ok_or_else(|| String::from("ready fixture omitted current fence"))?;
    let now = ready.service.recovery_now_millis();
    let store = ready
        .service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("ready fixture omitted recovery store"))?;
    store
        .revoke_lease(&old_lease.proof(), "test-replace")
        .map_err(|error| error.to_string())?;
    let lease = store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id: boot.deployment_id.clone(),
            instance_id: boot.instance_id.clone(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id: ready.service.config.caller_id.clone(),
            session_id: ready.service.config.session_id.clone(),
            now_millis: now,
            ttl_seconds: ready.service.config.recovery_ttl_seconds,
            renewal_interval_seconds: ready.service.config.recovery_renewal_interval_seconds,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant = super::super::host_lease_helpers::grant_value(
        &boot,
        &fence,
        &lease,
        &ready.service.config.caller_id,
        &ready.service.config.session_id,
    );
    let grant_digest = super::super::super::host_lease_control::grant_digest(&grant)
        .map_err(|error| format!("host grant could not be digested: {error:?}"))?;
    store
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            &fence.host_fence_id,
            fence.fence_generation,
            now + 1,
        )
        .map_err(|error| error.to_string())?;
    store
        .complete_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            1,
            &Uuid::new_v4().to_string(),
            now + 2,
        )
        .map_err(|error| error.to_string())?;
    ready.owner = store
        .current_continuation_owner(&ready.service.config.session_id, now + 3)
        .map_err(|error| error.to_string())?
        .owner
        .ok_or_else(|| String::from("replacement lease did not become current"))?;
    ready.service.recovery_lease = Some(lease.clone());
    ready.service.recovery_host_grant = Some(super::super::HostLeaseGrant {
        installation_id,
        grant_digest,
        grant,
    });
    ready.service.lease_active = true;
    ready.service.lease_revoked = false;
    ready.service.shutdown_requested = false;
    ready.service.recovery_lease_deadline = Some(Instant::now() + Duration::from_secs(30));
    ready.service.recovery_lease_deadline_lease_id = Some(lease.lease_id);
    if !ready
        .service
        .active_host_grant_matches(
            ready
                .service
                .recovery_lease
                .as_ref()
                .ok_or_else(|| String::from("replacement lease was not installed"))?,
        )
        .map_err(|error| error.to_string())?
    {
        return Err(String::from("replacement grant did not become ready"));
    }
    Ok(())
}
