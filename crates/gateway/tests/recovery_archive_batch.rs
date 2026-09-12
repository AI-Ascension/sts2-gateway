// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

use sts2_gateway::{
    GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryBootState, RecoveryEffectWitness,
    RecoveryIntentResult, RecoveryLease, RecoveryLeaseRequest, RecoveryOperationIntent,
    RecoveryOperationState, RecoveryReleaseSet, RecoveryStoreError, canonical_json_digest,
};

const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
const STATE: &str = "00000000-0000-4000-8000-000000000003";
const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CATALOG: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ARCHIVE_BATCH: u64 = 65;

fn path() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "sts2-recovery-archive-batch-{}-{nanos}.db",
        std::process::id()
    ))
}

fn ready_store(
    path: &std::path::Path,
) -> Result<(GatewayRecoveryStore, RecoveryLease), RecoveryStoreError> {
    let release = RecoveryReleaseSet::new(DIGEST, DIGEST, DIGEST, RUNTIME_V3_SCHEMA_DIGEST)?;
    let mut store = GatewayRecoveryStore::open(path)?;
    let boot = store.start_boot(DEPLOYMENT, INSTANCE, release, 1_000)?;
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
    lease: &RecoveryLease,
    operation_id: String,
) -> Result<RecoveryOperationIntent, RecoveryStoreError> {
    let canonical_json =
        br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#.to_vec();
    Ok(RecoveryOperationIntent {
        operation_id,
        deployment_id: lease.deployment_id.clone(),
        instance_id: lease.instance_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        boot_id: lease.boot_id.clone(),
        authority_generation: lease.authority_generation,
        lease_id: lease.lease_id.clone(),
        lease_epoch: lease.lease_epoch,
        schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
        payload_digest: canonical_json_digest(&canonical_json)?,
        canonical_json,
        expected_state_id: STATE.to_owned(),
        expected_generation: 0,
        catalog_digest: CATALOG.to_owned(),
        now_millis: 1_003,
    })
}

fn operation_id(index: u64) -> String {
    format!("00000000-0000-4000-8000-{index:012x}")
}

fn witness(
    lease: &RecoveryLease,
    host_fence_id: String,
    operation_id: String,
    payload_digest: String,
    index: u64,
) -> RecoveryEffectWitness {
    RecoveryEffectWitness {
        witness_id: format!("00000000-0000-4000-8001-{index:012x}"),
        operation_id,
        payload_digest,
        boot_id: lease.boot_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        host_fence_id,
        source: "host_game_thread".to_owned(),
        state_id: STATE.to_owned(),
        generation: 1,
        effect_digest: DIGEST.to_owned(),
        observed_at_millis: 1_005,
    }
}

#[test]
fn archive_batch_keeps_more_than_sixty_four_duplicate_tombstones() -> Result<(), RecoveryStoreError>
{
    let path = path();
    let (mut store, lease) = ready_store(&path)?;
    let proof = lease.proof();
    let fence = store.current_host_fence()?;

    for index in 1..=ARCHIVE_BATCH {
        let operation_id = operation_id(index);
        let original = intent(&lease, operation_id.clone())?;
        let payload_digest = original.payload_digest.clone();
        assert!(matches!(
            store.record_intent(&proof, original)?,
            RecoveryIntentResult::Created(_)
        ));
        store.mark_dispatched(&proof, INSTANCE, &operation_id, 1_004)?;
        store.record_outcome(
            INSTANCE,
            &operation_id,
            RecoveryOperationState::Settled,
            Some(200),
            None,
            Some(witness(
                &lease,
                fence.host_fence_id.clone(),
                operation_id.clone(),
                payload_digest,
                index,
            )),
            None,
            1_005,
        )?;
    }

    assert_eq!(
        store.archive_resolved_before(1_005, 86_401_005)?,
        ARCHIVE_BATCH as usize
    );
    assert_eq!(store.archived_count()?, ARCHIVE_BATCH as usize);
    assert_eq!(store.retained_count()?, ARCHIVE_BATCH as usize);
    for index in 1..=ARCHIVE_BATCH {
        assert!(matches!(
            store.record_intent(&proof, intent(&lease, operation_id(index))?)?,
            RecoveryIntentResult::Duplicate(_)
        ));
    }

    let mut conflict = intent(&lease, operation_id(ARCHIVE_BATCH))?;
    conflict.canonical_json =
        br#"{"action":{"kind":"proceed"},"action_id":"action-proceed"}"#.to_vec();
    conflict.payload_digest = canonical_json_digest(&conflict.canonical_json)?;
    assert_eq!(
        store.record_intent(&proof, conflict),
        Err(RecoveryStoreError::OperationConflict)
    );
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}
