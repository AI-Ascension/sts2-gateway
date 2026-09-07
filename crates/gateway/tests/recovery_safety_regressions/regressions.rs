// SPDX-License-Identifier: MIT

//! Each test below must assert a safe rejection or monotonic result.  At the
//! reviewed baseline every Linux test is expected to fail; no test is ignored.

use sts2_gateway::{
    GatewayRecoveryStore, RecoveryLeaseRequest, RecoveryOperationState, RecoveryStoreConfig,
    RecoveryStoreError, RecoveryUncertaintyReason, canonical_json_digest,
};

use super::support::*;

#[test]
fn revoked_lease_cannot_record_intent() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("revoked-lease");
    let (mut store, lease, _) = ready_store(&path)?;
    let proof = lease.proof();
    store.revoke_lease(&proof, "operator")?;
    let result = store.record_intent(
        &proof,
        intent(&lease, "00000000-0000-4000-8000-000000000010")?,
    );
    assert!(
        matches!(result, Err(RecoveryStoreError::LeaseRevoked)),
        "a revoked lease must fail closed before recording an intent: {result:?}"
    );
    Ok(())
}

#[test]
fn terminal_operation_invalidates_admission_ticket() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("terminal-ticket");
    let (mut store, lease, fence) = ready_store(&path)?;
    let proof = lease.proof();
    let operation_id = "00000000-0000-4000-8000-000000000011";
    created_operation(&mut store, &lease, operation_id)?;
    let payload_digest =
        canonical_json_digest(br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#)?;
    let ticket = store.issue_admission_ticket(
        &proof,
        &fence,
        INSTANCE,
        operation_id,
        &payload_digest,
        1_005,
        1,
    )?;
    let settled = store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Settled,
        Some(200),
        None,
        Some(witness(&lease, &fence, operation_id, &fence.host_fence_id)?),
        None,
        1_006,
    )?;
    assert_eq!(settled.state, RecoveryOperationState::Settled);
    let validation = store.validate_admission_ticket(&proof, &fence, &ticket.ticket_id, 1_007);
    assert!(
        validation.is_err(),
        "a ticket for a terminal operation must not remain admissible: {validation:?}"
    );
    Ok(())
}

#[test]
fn witness_host_fence_identity_must_match_current_fence() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("witness-fence");
    let (mut store, lease, fence) = ready_store(&path)?;
    let operation_id = "00000000-0000-4000-8000-000000000012";
    created_operation(&mut store, &lease, operation_id)?;
    let result = store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Settled,
        Some(200),
        None,
        Some(witness(
            &lease,
            &fence,
            operation_id,
            "00000000-0000-4000-8000-000000000098",
        )?),
        None,
        1_005,
    );
    assert!(
        result.is_err(),
        "a witness from another host fence must not settle this operation: {result:?}"
    );
    Ok(())
}

#[test]
fn unwitnessed_reconciled_outcome_is_rejected() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("unwitnessed-reconcile");
    let (mut store, lease, _) = ready_store(&path)?;
    let operation_id = "00000000-0000-4000-8000-000000000013";
    created_operation(&mut store, &lease, operation_id)?;
    store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Unknown,
        None,
        None,
        None,
        Some(RecoveryUncertaintyReason::Timeout),
        1_005,
    )?;
    let result = store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Reconciled,
        None,
        None,
        None,
        None,
        1_006,
    );
    assert!(
        matches!(result, Err(RecoveryStoreError::ContractMismatch(_))),
        "reconciliation must carry an authoritative effect witness: {result:?}"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_alias_cannot_bypass_recovery_lock() -> Result<(), RecoveryStoreError> {
    use std::os::unix::fs::symlink;

    let mut artifacts = TempArtifacts::new();
    let target = artifacts.path("symlink-target");
    let alias = artifacts.add(target.with_extension("symlink.db"));
    let first = GatewayRecoveryStore::open(&target)?;
    symlink(&target, &alias)
        .map_err(|error| RecoveryStoreError::Io(format!("symlink setup failed: {error}")))?;
    let second = GatewayRecoveryStore::open(&alias);
    assert!(
        second.is_err(),
        "a symlink alias must not acquire a second authority lock: {second:?}"
    );
    drop(first);
    Ok(())
}

#[test]
fn failed_restore_does_not_leave_a_usable_clone() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let source = artifacts.path("failed-restore-source");
    let backup = artifacts.path("failed-restore-backup");
    let destination = artifacts.path("failed-restore-destination");
    let (store, _lease, _fence) = ready_store(&source)?;
    store.backup_to(&backup)?;
    drop(store);
    let result = GatewayRecoveryStore::restore_rekey(
        &backup,
        &destination,
        DEPLOYMENT,
        INSTANCE,
        release(OTHER_DIGEST)?,
        2_000,
    );
    let rejected_for_release = matches!(&result, Err(RecoveryStoreError::ReleaseMismatch));
    drop(result);
    assert!(
        rejected_for_release,
        "a restore with mismatched release metadata must be rejected"
    );
    let safe_destination = if !destination.exists() {
        true
    } else {
        match GatewayRecoveryStore::open(&destination) {
            Ok(store) => {
                drop(store);
                false
            }
            Err(_) => true,
        }
    };
    assert!(
        safe_destination,
        "failed restore must remove or quarantine the destination before it is usable"
    );
    Ok(())
}

#[test]
fn restore_generation_must_not_roll_back_live_authority() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let source = artifacts.path("rollback-source");
    let backup = artifacts.path("rollback-backup");
    let destination = artifacts.path("rollback-destination");
    let (mut live_store, _lease, _fence) = ready_store(&source)?;
    live_store.backup_to(&backup)?;
    for now_millis in [2_000, 3_000, 4_000, 5_000] {
        live_store.start_boot(DEPLOYMENT, INSTANCE, release(DIGEST)?, now_millis)?;
    }
    let live_generation = live_store.current_boot()?.authority_generation;
    drop(live_store);
    let result = GatewayRecoveryStore::restore_rekey(
        &backup,
        &destination,
        DEPLOYMENT,
        INSTANCE,
        release(DIGEST)?,
        6_000,
    );
    let safe = match result {
        Err(_) => true,
        Ok((store, boot)) => {
            let monotonic = boot.authority_generation > live_generation;
            drop(store);
            monotonic
        }
    };
    assert!(
        safe,
        "restoring an older backup must reject or establish generation above live authority {live_generation}"
    );
    Ok(())
}

#[test]
fn lease_expiry_must_not_regress_when_audit_clock_moves_backwards() -> Result<(), RecoveryStoreError>
{
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("rollback-expiry");
    let (mut store, lease, _) = ready_store(&path)?;
    let proof = lease.proof();
    let first = store.renew_lease(&proof, 1, 2_000)?;
    let second = store.renew_lease(&proof, 2, 1_000);
    let safe = match &second {
        Err(_) => true,
        Ok(renewed) => renewed.expires_at_millis >= first.expires_at_millis,
    };
    assert!(
        safe,
        "lease expiry must remain monotonic across clock rollback; first={:?} second={:?}",
        first.expires_at_millis, second
    );
    Ok(())
}

#[test]
fn configured_lease_ttl_and_renewal_policy_must_be_enforced() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("ttl-policy");
    let config = RecoveryStoreConfig {
        unresolved_capacity: 2,
        retained_capacity: 2,
        lease_ttl_seconds: 5,
        lease_renewal_interval_seconds: 1,
    };
    let mut store = GatewayRecoveryStore::open_with_config(&path, config.clone())?;
    let boot = store.start_boot(DEPLOYMENT, INSTANCE, release(DIGEST)?, 1_000)?;
    let fence = store.complete_host_fence(&boot, 1_001)?;
    let result = store.acquire_lease(RecoveryLeaseRequest {
        deployment_id: DEPLOYMENT.to_owned(),
        instance_id: INSTANCE.to_owned(),
        instance_incarnation: boot.instance_incarnation,
        boot_id: boot.boot_id,
        authority_generation: boot.authority_generation,
        host_fence_id: fence.host_fence_id,
        host_fence_generation: fence.fence_generation,
        caller_id: "caller-1".to_owned(),
        session_id: "session-1".to_owned(),
        now_millis: 1_002,
        ttl_seconds: 300,
        renewal_interval_seconds: 299,
    });
    let safe = match result {
        Err(RecoveryStoreError::InvalidInput(_)) => true,
        Ok(lease) => {
            lease.ttl_seconds <= config.lease_ttl_seconds
                && lease.renewal_interval_seconds <= config.lease_renewal_interval_seconds
        }
        Err(_) => false,
    };
    assert!(
        safe,
        "configured lease policy must cap or reject request ttl/renewal (config={config:?})"
    );
    Ok(())
}
