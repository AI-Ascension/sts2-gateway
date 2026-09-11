// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

use sts2_gateway::{
    GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryBootState, RecoveryEffectWitness,
    RecoveryIntentResult, RecoveryLeaseRequest, RecoveryOperationIntent, RecoveryOperationState,
    RecoveryReleaseSet, RecoveryStoreError, RecoveryTicketState, RecoveryUncertaintyReason,
    canonical_json_digest, canonicalize_recovery_action,
};

const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
const STATE: &str = "00000000-0000-4000-8000-000000000003";
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CATALOG: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn path(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "sts2-recovery-{label}-{}-{nanos}.db",
        std::process::id()
    ))
}

fn release() -> Result<RecoveryReleaseSet, RecoveryStoreError> {
    RecoveryReleaseSet::new(DIGEST, DIGEST, DIGEST, RUNTIME_V3_SCHEMA_DIGEST)
}

fn ready_store(
    path: &std::path::Path,
) -> Result<(GatewayRecoveryStore, sts2_gateway::RecoveryLease), RecoveryStoreError> {
    let mut store = GatewayRecoveryStore::open(path)?;
    let boot = store.start_boot(DEPLOYMENT, INSTANCE, release()?, 1_000)?;
    assert_eq!(boot.state, RecoveryBootState::FenceRequired);
    let fence = store.complete_host_fence(&boot, 1_001)?;
    let lease = store.acquire_lease(RecoveryLeaseRequest {
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
        ttl_seconds: 30,
        renewal_interval_seconds: 10,
    })?;
    Ok((store, lease))
}

fn intent(
    lease: &sts2_gateway::RecoveryLease,
    operation_id: &str,
) -> Result<RecoveryOperationIntent, RecoveryStoreError> {
    let action = br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#;
    Ok(RecoveryOperationIntent {
        operation_id: operation_id.to_owned(),
        deployment_id: lease.deployment_id.clone(),
        instance_id: lease.instance_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        boot_id: lease.boot_id.clone(),
        authority_generation: lease.authority_generation,
        lease_id: lease.lease_id.clone(),
        lease_epoch: lease.lease_epoch,
        schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
        canonical_json: action.to_vec(),
        payload_digest: canonical_json_digest(action)?,
        expected_state_id: STATE.to_owned(),
        expected_generation: 0,
        catalog_digest: CATALOG.to_owned(),
        now_millis: 1_003,
    })
}

#[test]
fn rcj_vectors_reject_noncanonical_and_unsupported_actions() -> Result<(), RecoveryStoreError> {
    let action = br#"{"action":{"character_id":"ironclad","kind":"start_run"},"action_id":"action-start-run"}"#;
    assert_eq!(canonicalize_recovery_action(action)?, action);
    assert_eq!(
        canonical_json_digest(action)?,
        "6464c6d2571ce0c697b147c1554625e1128d730f131969b52c5c2d85ccea6c1c"
    );
    for malformed in [
        br#"{"action":{"kind":"end_turn"},"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#.as_slice(),
        br#"{"action":{"kind":"end_turn"},"action_id":"action\u002dend-turn"}"#.as_slice(),
        br#"{"action":{"kind":"unknown"},"action_id":"action-unknown"}"#.as_slice(),
    ] {
        assert!(canonicalize_recovery_action(malformed).is_err());
    }
    Ok(())
}

#[test]
fn boot_fence_lease_and_unknown_survive_restart() -> Result<(), RecoveryStoreError> {
    let path = path("restart");
    let (mut store, lease) = ready_store(&path)?;
    assert_eq!(store.pragmas()?.journal_mode, "wal");
    assert_eq!(store.pragmas()?.synchronous, 2);
    let proof = lease.proof();
    let operation = match store.record_intent(
        &proof,
        intent(&lease, "00000000-0000-4000-8000-000000000010")?,
    )? {
        RecoveryIntentResult::Created(operation) => operation,
        RecoveryIntentResult::Duplicate(_) => return Err(RecoveryStoreError::OperationConflict),
    };
    store.mark_dispatched(&proof, INSTANCE, &operation.operation_id, 1_004)?;
    store.record_outcome(
        INSTANCE,
        &operation.operation_id,
        RecoveryOperationState::Unknown,
        None,
        None,
        None,
        Some(RecoveryUncertaintyReason::Timeout),
        1_005,
    )?;
    drop(lease);
    drop(store);

    let mut reopened = GatewayRecoveryStore::open(&path)?;
    let old_boot = reopened.current_boot()?;
    let replacement = reopened.start_boot(DEPLOYMENT, INSTANCE, release()?, 2_000)?;
    assert!(replacement.authority_generation > old_boot.authority_generation);
    assert_ne!(replacement.boot_id, old_boot.boot_id);
    assert_eq!(reopened.unresolved_count()?, 1);
    let Some(recovered) = reopened.lookup_operation(
        INSTANCE,
        "00000000-0000-4000-8000-000000000010",
        &canonical_json_digest(br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#)?,
    )?
    else {
        return Err(RecoveryStoreError::OperationNotFound);
    };
    assert_eq!(recovered.state, RecoveryOperationState::Unknown);
    assert_eq!(
        recovered.uncertainty_reason,
        Some(RecoveryUncertaintyReason::AuthorityRotated)
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn duplicate_and_conflict_never_send_a_second_intent() -> Result<(), RecoveryStoreError> {
    let path = path("duplicate");
    let (mut store, lease) = ready_store(&path)?;
    let proof = lease.proof();
    let operation_id = "00000000-0000-4000-8000-000000000011";
    let first = store.record_intent(&proof, intent(&lease, operation_id)?)?;
    let second = store.record_intent(&proof, intent(&lease, operation_id)?)?;
    assert!(matches!(first, RecoveryIntentResult::Created(_)));
    assert!(matches!(second, RecoveryIntentResult::Duplicate(_)));
    let mut conflicting = intent(&lease, operation_id)?;
    conflicting.canonical_json =
        br#"{"action":{"kind":"proceed"},"action_id":"action-proceed"}"#.to_vec();
    conflicting.payload_digest = canonical_json_digest(&conflicting.canonical_json)?;
    assert_eq!(
        store.record_intent(&proof, conflicting),
        Err(RecoveryStoreError::OperationConflict)
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn settled_requires_operation_bound_witness() -> Result<(), RecoveryStoreError> {
    let path = path("witness");
    let (mut store, lease) = ready_store(&path)?;
    let proof = lease.proof();
    let operation_id = "00000000-0000-4000-8000-000000000012";
    store.record_intent(&proof, intent(&lease, operation_id)?)?;
    store.mark_dispatched(&proof, INSTANCE, operation_id, 1_004)?;
    let witness = RecoveryEffectWitness {
        witness_id: "00000000-0000-4000-8000-000000000013".to_owned(),
        operation_id: operation_id.to_owned(),
        payload_digest: canonical_json_digest(
            br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#,
        )?,
        boot_id: lease.boot_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        host_fence_id: store.current_host_fence()?.host_fence_id,
        source: "host_game_thread".to_owned(),
        state_id: STATE.to_owned(),
        generation: 1,
        effect_digest: DIGEST.to_owned(),
        observed_at_millis: 1_005,
    };
    assert_eq!(
        store.record_outcome(
            INSTANCE,
            operation_id,
            RecoveryOperationState::Settled,
            None,
            None,
            None,
            None,
            1_005
        ),
        Err(RecoveryStoreError::ContractMismatch(
            "settled operation requires an operation-specific witness".to_owned()
        ))
    );
    let settled = store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Settled,
        Some(200),
        None,
        Some(witness),
        None,
        1_005,
    )?;
    assert_eq!(settled.state, RecoveryOperationState::Settled);
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn admission_ticket_is_fenced_and_expires_atomically() -> Result<(), RecoveryStoreError> {
    let path = path("ticket");
    let (mut store, lease) = ready_store(&path)?;
    let proof = lease.proof();
    let operation_id = "00000000-0000-4000-8000-000000000014";
    let operation = match store.record_intent(&proof, intent(&lease, operation_id)?)? {
        RecoveryIntentResult::Created(operation) => operation,
        RecoveryIntentResult::Duplicate(_) => return Err(RecoveryStoreError::OperationConflict),
    };
    store.mark_dispatched(&proof, INSTANCE, operation_id, 1_004)?;
    let fence = store.current_host_fence()?;
    let ticket = store.issue_admission_ticket(
        &proof,
        &fence,
        INSTANCE,
        operation_id,
        &operation.payload_digest,
        1_005,
        1,
    )?;
    assert_eq!(ticket.state, RecoveryTicketState::Issued);
    let admitted = store.transition_admission_ticket(
        &proof,
        &fence,
        &ticket.ticket_id,
        RecoveryTicketState::Admitted,
        1_006,
    )?;
    assert_eq!(admitted.state, RecoveryTicketState::Admitted);
    assert_eq!(store.expire_admission_tickets(2_005)?, 1);
    assert_eq!(
        store.validate_admission_ticket(&proof, &fence, &ticket.ticket_id, 2_005),
        Err(RecoveryStoreError::AdmissionTicketExpired)
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn resolved_archive_preserves_duplicate_tombstone() -> Result<(), RecoveryStoreError> {
    let path = path("archive");
    let (mut store, lease) = ready_store(&path)?;
    let proof = lease.proof();
    let operation_id = "00000000-0000-4000-8000-000000000015";
    let original = intent(&lease, operation_id)?;
    let payload_digest = original.payload_digest.clone();
    store.record_intent(&proof, original)?;
    store.mark_dispatched(&proof, INSTANCE, operation_id, 1_004)?;
    let fence = store.current_host_fence()?;
    store.record_outcome(
        INSTANCE,
        operation_id,
        RecoveryOperationState::Settled,
        Some(200),
        None,
        Some(RecoveryEffectWitness {
            witness_id: "00000000-0000-4000-8000-000000000016".to_owned(),
            operation_id: operation_id.to_owned(),
            payload_digest: payload_digest.clone(),
            boot_id: lease.boot_id.clone(),
            instance_incarnation: lease.instance_incarnation.clone(),
            host_fence_id: fence.host_fence_id,
            source: "host_game_thread".to_owned(),
            state_id: STATE.to_owned(),
            generation: 1,
            effect_digest: DIGEST.to_owned(),
            observed_at_millis: 1_005,
        }),
        None,
        1_005,
    )?;
    assert_eq!(store.archive_resolved_before(1_005, 86_401_005)?, 1);
    assert_eq!(store.archived_count()?, 1);
    assert_eq!(store.retained_count()?, 1);
    let duplicate = store.record_intent(&proof, intent(&lease, operation_id)?)?;
    assert!(matches!(duplicate, RecoveryIntentResult::Duplicate(_)));
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn backup_restore_rekeys_before_fence() -> Result<(), RecoveryStoreError> {
    let source = path("backup-source");
    let backup = path("backup-copy");
    let restored = path("backup-restored");
    let (mut store, lease) = ready_store(&source)?;
    let fence = store.current_host_fence()?;
    let installation_id = "00000000-0000-4000-8000-000000000017";
    let grant_digest = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let ack_message_id = "00000000-0000-4000-8000-000000000018";
    store.prepare_host_lease_install(
        &lease.lease_id,
        installation_id,
        grant_digest,
        &fence.host_fence_id,
        fence.fence_generation,
        1_003,
    )?;
    store.complete_host_lease_install(
        &lease.lease_id,
        installation_id,
        grant_digest,
        1,
        ack_message_id,
        1_004,
    )?;
    assert!(store.host_lease_is_ready(&lease.lease_id)?);
    store.backup_to(&backup)?;
    drop(store);
    let (mut restored_store, boot) = GatewayRecoveryStore::restore_rekey(
        &backup,
        &restored,
        "00000000-0000-4000-8000-000000000099",
        INSTANCE,
        release()?,
        10_000,
    )?;
    assert_eq!(boot.state, RecoveryBootState::FenceRequired);
    assert_eq!(boot.deployment_id, "00000000-0000-4000-8000-000000000099");
    assert!(boot.authority_generation >= 2);
    assert!(restored_store.current_host_fence().is_err());
    let binding = restored_store
        .host_lease_binding(&lease.lease_id)?
        .ok_or(RecoveryStoreError::LeaseNotFound)?;
    assert_eq!(
        binding.state,
        sts2_gateway::RecoveryHostLeaseState::RestartInvalidated
    );
    assert!(!restored_store.host_lease_is_ready(&lease.lease_id)?);
    restored_store.complete_host_fence(&boot, 10_001)?;
    drop(restored_store);
    for target in [&source, &backup, &restored] {
        let _ = std::fs::remove_file(target);
        let _ = std::fs::remove_file(target.with_extension("gateway-recovery.lock"));
    }
    Ok(())
}
