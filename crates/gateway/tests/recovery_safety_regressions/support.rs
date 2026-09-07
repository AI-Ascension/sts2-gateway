// SPDX-License-Identifier: MIT

//! Shared valid recovery context for the red public-API regressions.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_gateway::{
    GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryBootState, RecoveryEffectWitness,
    RecoveryHostFence, RecoveryIntentResult, RecoveryLease, RecoveryLeaseRequest,
    RecoveryOperationIntent, RecoveryReleaseSet, RecoveryStoreError, canonical_json_digest,
};

pub(crate) const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
pub(crate) const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
pub(crate) const STATE: &str = "00000000-0000-4000-8000-000000000003";
pub(crate) const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) const OTHER_DIGEST: &str =
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
pub(crate) const CATALOG: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempArtifacts {
    paths: Vec<PathBuf>,
}

impl TempArtifacts {
    pub(crate) fn new() -> Self {
        Self { paths: Vec::new() }
    }

    pub(crate) fn path(&mut self, label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sts2-recovery-regression-{label}-{}-{now}-{sequence}.db",
            std::process::id()
        ));
        self.paths.push(path.clone());
        path
    }

    pub(crate) fn add(&mut self, path: PathBuf) -> PathBuf {
        self.paths.push(path.clone());
        path
    }
}

impl Drop for TempArtifacts {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
        }
    }
}

pub(crate) fn release(digest: &str) -> Result<RecoveryReleaseSet, RecoveryStoreError> {
    RecoveryReleaseSet::new(digest, digest, digest, RUNTIME_V3_SCHEMA_DIGEST)
}

pub(crate) fn ready_store(
    path: &Path,
) -> Result<(GatewayRecoveryStore, RecoveryLease, RecoveryHostFence), RecoveryStoreError> {
    let mut store = GatewayRecoveryStore::open(path)?;
    let boot = store.start_boot(DEPLOYMENT, INSTANCE, release(DIGEST)?, 1_000)?;
    assert_eq!(boot.state, RecoveryBootState::FenceRequired);
    let fence = store.complete_host_fence(&boot, 1_001)?;
    let lease = store.acquire_lease(RecoveryLeaseRequest {
        deployment_id: DEPLOYMENT.to_owned(),
        instance_id: INSTANCE.to_owned(),
        instance_incarnation: boot.instance_incarnation,
        boot_id: boot.boot_id,
        authority_generation: boot.authority_generation,
        host_fence_id: fence.host_fence_id.clone(),
        host_fence_generation: fence.fence_generation,
        caller_id: "caller-1".to_owned(),
        session_id: "session-1".to_owned(),
        now_millis: 1_002,
        ttl_seconds: 30,
        renewal_interval_seconds: 10,
    })?;
    Ok((store, lease, fence))
}

pub(crate) fn intent(
    lease: &RecoveryLease,
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

pub(crate) fn witness(
    lease: &RecoveryLease,
    fence: &RecoveryHostFence,
    operation_id: &str,
    host_fence_id: &str,
) -> Result<RecoveryEffectWitness, RecoveryStoreError> {
    Ok(RecoveryEffectWitness {
        witness_id: "00000000-0000-4000-8000-000000000099".to_owned(),
        operation_id: operation_id.to_owned(),
        payload_digest: canonical_json_digest(
            br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#,
        )?,
        boot_id: lease.boot_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        host_fence_id: host_fence_id.to_owned(),
        source: "host_game_thread".to_owned(),
        state_id: STATE.to_owned(),
        generation: fence.fence_generation,
        effect_digest: DIGEST.to_owned(),
        observed_at_millis: 1_005,
    })
}

pub(crate) fn created_operation(
    store: &mut GatewayRecoveryStore,
    lease: &RecoveryLease,
    operation_id: &str,
) -> Result<(), RecoveryStoreError> {
    let proof = lease.proof();
    match store.record_intent(&proof, intent(lease, operation_id)?)? {
        RecoveryIntentResult::Created(_) => {}
        RecoveryIntentResult::Duplicate(_) => return Err(RecoveryStoreError::OperationConflict),
    }
    store.mark_dispatched(&proof, INSTANCE, operation_id, 1_004)?;
    Ok(())
}
