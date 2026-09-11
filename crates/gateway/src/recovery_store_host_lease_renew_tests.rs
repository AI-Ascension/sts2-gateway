// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::super::super::recovery_types::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryHostLeaseState, RecoveryReleaseSet, RecoveryStoreError,
};
use super::super::{GatewayRecoveryStore, RecoveryLeaseRequest};

const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "sts2-gateway-{label}-{}-{}.db",
            std::process::id(),
            Uuid::new_v4()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("gateway-recovery.lock"));
        let _ = std::fs::remove_file(self.0.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.0.with_extension("db-shm"));
    }
}

fn ready_store(
    database: &TempDatabase,
) -> Result<
    (
        GatewayRecoveryStore,
        super::super::super::recovery_types::RecoveryLease,
    ),
    RecoveryStoreError,
> {
    let mut store = GatewayRecoveryStore::open(database.path())?;
    let digest = "a".repeat(64);
    let boot = store.start_boot(
        DEPLOYMENT,
        INSTANCE,
        RecoveryReleaseSet::new(&digest, &digest, &digest, RUNTIME_V3_SCHEMA_DIGEST)?,
        1_000,
    )?;
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

#[test]
fn suppressed_install_transition_is_rejected_before_host_effect() -> Result<(), RecoveryStoreError>
{
    let database = TempDatabase::new("install-transition");
    let (mut store, lease) = ready_store(&database)?;
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER suppress_host_lease_install
             BEFORE UPDATE OF host_state ON leases
             WHEN NEW.host_state = 'PENDING_HOST_INSTALL'
             BEGIN SELECT RAISE(IGNORE); END;",
        )
        .map_err(super::super::map_sql_error)?;

    let installation_id = Uuid::new_v4().to_string();
    let digest = "b".repeat(64);
    let result = store.prepare_host_lease_install(
        &lease.lease_id,
        &installation_id,
        &digest,
        &store.current_host_fence()?.host_fence_id,
        1,
        1_003,
    );
    assert!(
        matches!(result, Err(RecoveryStoreError::StaleLease)),
        "a suppressed install intent must fail closed: {result:?}"
    );
    let binding = store
        .host_lease_binding(&lease.lease_id)?
        .ok_or(RecoveryStoreError::LeaseNotFound)?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Uninstalled);
    assert!(binding.installation_id.is_none());
    Ok(())
}

#[test]
fn suppressed_renew_transition_is_rejected_before_host_effect() -> Result<(), RecoveryStoreError> {
    let database = TempDatabase::new("renew-transition");
    let (mut store, lease) = ready_store(&database)?;
    let fence = store.current_host_fence()?;
    let installation_id = Uuid::new_v4().to_string();
    let digest = "b".repeat(64);
    store.prepare_host_lease_install(
        &lease.lease_id,
        &installation_id,
        &digest,
        &fence.host_fence_id,
        fence.fence_generation,
        1_003,
    )?;
    store.complete_host_lease_install(
        &lease.lease_id,
        &installation_id,
        &digest,
        1,
        &Uuid::new_v4().to_string(),
        1_004,
    )?;
    store
        .conn
        .execute_batch(
            "CREATE TRIGGER suppress_host_lease_renew
             BEFORE UPDATE OF host_state ON leases
             WHEN NEW.host_state = 'PENDING_HOST_RENEW'
             BEGIN SELECT RAISE(IGNORE); END;",
        )
        .map_err(super::super::map_sql_error)?;

    let result = store.prepare_host_lease_renew(&lease.proof(), 1, 1_005);
    assert!(
        matches!(result, Err(RecoveryStoreError::StaleLease)),
        "a suppressed renewal intent must fail closed: {result:?}"
    );
    let binding = store
        .host_lease_binding(&lease.lease_id)?
        .ok_or(RecoveryStoreError::LeaseNotFound)?;
    assert_eq!(binding.state, RecoveryHostLeaseState::Installed);
    assert_eq!(binding.host_renew_sequence, 0);
    Ok(())
}
