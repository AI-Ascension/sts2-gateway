// SPDX-License-Identifier: MIT

#[test]
fn missing_live_owner_memory_is_unknown_and_cannot_be_claimed() -> Result<(), String> {
    let mut ready = ready_service()?;
    ready.service.recovery_lease = None;
    ready.service.lease_active = false;
    ready.service.recovery_lease_deadline = None;
    let read = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&read);
    assert_eq!(status, 200);
    let current = response_value(&body)?;
    assert_eq!(current["payload"]["state"], "unknown");

    let claim = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": Uuid::new_v4().to_string(),
            "expected_owner": serde_json::to_value(&ready.owner).map_err(|e| e.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&claim);
    assert_eq!(status, 503);
    assert_eq!(
        response_value(&body)?["error_code"],
        "continuation_owner_unknown"
    );
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

#[test]
fn missing_installed_grant_is_unknown_and_cannot_be_claimed() -> Result<(), String> {
    let mut ready = ready_service()?;
    ready.service.recovery_host_grant = None;
    assert_unknown_and_claim_refused(&mut ready)?;
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

#[test]
fn stale_installed_grant_is_unknown_and_cannot_be_claimed() -> Result<(), String> {
    let mut ready = ready_service()?;
    let cached = ready
        .service
        .recovery_host_grant
        .as_mut()
        .ok_or_else(|| String::from("ready fixture omitted its installed grant"))?;
    cached.grant_digest = "f".repeat(64);
    assert_unknown_and_claim_refused(&mut ready)?;
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

#[test]
fn current_owner_requires_matching_in_memory_boot_lineage() -> Result<(), String> {
    let mut ready = ready_service()?;
    let boot = ready
        .service
        .recovery_boot
        .as_mut()
        .ok_or_else(|| String::from("ready fixture omitted its current boot"))?;
    boot.boot_id = Uuid::new_v4().to_string();
    let read = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&read);
    assert_eq!(status, 200);
    assert_eq!(response_value(&body)?["payload"]["state"], "unknown");
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

fn assert_unknown_and_claim_refused(ready: &mut ReadyService) -> Result<(), String> {
    let read = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&read);
    assert_eq!(status, 200);
    assert_eq!(response_value(&body)?["payload"]["state"], "unknown");

    let operation_id = Uuid::new_v4().to_string();
    let claim = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&ready.owner).map_err(|error| error.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&claim);
    assert_eq!(status, 503);
    assert_eq!(
        response_value(&body)?["error_code"],
        "continuation_owner_unknown"
    );
    let stored = ready
        .service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("ready fixture lost recovery store"))?
        .lookup_continuation_owner_claim(&operation_id)
        .map_err(|error| error.to_string())?;
    assert!(stored.is_none());
    Ok(())
}
