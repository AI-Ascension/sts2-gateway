// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use rusqlite::Connection;
use sts2_gateway::{
    GatewayRecoveryStore, RecoveryContinuationOwnerClaimResult, RecoveryContinuationOwnerState,
    RecoveryLeaseRequest, RecoveryReleaseSet, RecoveryStoreError,
};
use uuid::Uuid;

fn store_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sts2-gateway-continuation-owner-{label}-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ))
}

fn ready_owner(path: &PathBuf) -> Result<(GatewayRecoveryStore, String, String, u64), String> {
    let mut store = GatewayRecoveryStore::open(path).map_err(|error| error.to_string())?;
    let deployment_id = Uuid::new_v4().to_string();
    let instance_id = Uuid::new_v4().to_string();
    let caller_id = Uuid::new_v4().to_string();
    let session_id = format!("session-{}", Uuid::new_v4());
    let now = 1_800_000_000_000;
    let boot = store
        .start_boot(
            &deployment_id,
            &instance_id,
            RecoveryReleaseSet::unconfigured(),
            now,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, now + 1)
        .map_err(|error| error.to_string())?;
    let lease = store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id,
            instance_id,
            instance_incarnation: boot.instance_incarnation,
            boot_id: boot.boot_id,
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id,
            session_id: session_id.clone(),
            now_millis: now + 2,
            ttl_seconds: 30,
            renewal_interval_seconds: 10,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant_digest = "a".repeat(64);
    store
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            &fence.host_fence_id,
            fence.fence_generation,
            now + 3,
        )
        .map_err(|error| error.to_string())?;
    store
        .complete_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            1,
            &Uuid::new_v4().to_string(),
            now + 4,
        )
        .map_err(|error| error.to_string())?;
    Ok((store, lease.lease_id, session_id, now))
}

#[test]
fn schema_v3_migrates_the_claim_journal_without_existing_rows() -> Result<(), String> {
    let path = store_path("migration");
    let connection = Connection::open(&path).map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA user_version = 3;")
        .map_err(|error| error.to_string())?;
    drop(connection);

    let store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    assert_eq!(
        store
            .lookup_continuation_owner_claim(&Uuid::new_v4().to_string())
            .map_err(|error| error.to_string())?,
        None
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn current_owner_is_non_secret_and_claim_is_idempotent_and_fenced() -> Result<(), String> {
    let path = store_path("claim");
    let (mut store, lease_id, session_id, now) = ready_owner(&path)?;
    let snapshot = store
        .current_continuation_owner(&session_id, now + 5)
        .map_err(|error| error.to_string())?;
    assert_eq!(snapshot.state, RecoveryContinuationOwnerState::Available);
    let owner = snapshot.owner.ok_or("owner snapshot missing")?;
    assert_eq!(owner.lease_id, lease_id);

    let operation_id = Uuid::new_v4().to_string();
    let first = store
        .claim_continuation_owner(&operation_id, &owner, now + 6)
        .map_err(|error| error.to_string())?;
    let RecoveryContinuationOwnerClaimResult::Created(first) = first else {
        return Err(String::from("first owner claim was not created"));
    };
    let duplicate = store
        .claim_continuation_owner(&operation_id, &owner, now + 7)
        .map_err(|error| error.to_string())?;
    let RecoveryContinuationOwnerClaimResult::Duplicate(duplicate) = duplicate else {
        return Err(String::from("repeated owner claim was not a duplicate"));
    };
    assert_eq!(first, duplicate);

    let sibling_operation = Uuid::new_v4().to_string();
    assert_eq!(
        store.claim_continuation_owner(&sibling_operation, &owner, now + 8),
        Err(RecoveryStoreError::OperationConflict)
    );

    let mut stale_owner = owner;
    stale_owner.lease_epoch += 1;
    assert_eq!(
        store.claim_continuation_owner(&Uuid::new_v4().to_string(), &stale_owner, now + 9),
        Err(RecoveryStoreError::StaleLease)
    );

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}

#[test]
fn expiry_and_incomplete_host_install_never_appear_available() -> Result<(), String> {
    let path = store_path("expiry");
    let (store, _lease_id, session_id, now) = ready_owner(&path)?;
    let expired = store
        .current_continuation_owner(&session_id, now + 40_000)
        .map_err(|error| error.to_string())?;
    assert_eq!(expired.state, RecoveryContinuationOwnerState::Expired);
    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));

    let path = store_path("uninstalled");
    let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let deployment_id = Uuid::new_v4().to_string();
    let instance_id = Uuid::new_v4().to_string();
    let boot = store
        .start_boot(
            &deployment_id,
            &instance_id,
            RecoveryReleaseSet::unconfigured(),
            now,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, now + 1)
        .map_err(|error| error.to_string())?;
    store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id,
            instance_id,
            instance_incarnation: boot.instance_incarnation,
            boot_id: boot.boot_id,
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id,
            host_fence_generation: fence.fence_generation,
            caller_id: Uuid::new_v4().to_string(),
            session_id: session_id.clone(),
            now_millis: now + 2,
            ttl_seconds: 30,
            renewal_interval_seconds: 10,
        })
        .map_err(|error| error.to_string())?;
    let pending = store
        .current_continuation_owner(&session_id, now + 3)
        .map_err(|error| error.to_string())?;
    assert_eq!(pending.state, RecoveryContinuationOwnerState::Unknown);

    drop(store);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}
