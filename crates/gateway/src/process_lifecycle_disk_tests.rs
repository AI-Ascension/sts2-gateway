// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, DeterministicLeaseDecision, FakeClock, FakeProcess, InstanceId,
    LifecycleOperationState, LifecycleRequest, ProcessLifecycle, ProcessLifecycleConfig, profiles,
};
use crate::{LifecycleError, LifecycleStoreError, OperationId, SqliteLifecycleStore};

fn unique_store_path(label: &str) -> std::path::PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "sts2-gateway-{label}-{}-{nonce}.sqlite",
        std::process::id()
    ))
}

fn remove_store_files(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lifecycle.lock");
    let _ = std::fs::remove_file(std::path::PathBuf::from(lock_path));
}

fn remove_coordinator_lock(path: &std::path::Path) {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lifecycle.lock");
    let _ = std::fs::remove_file(std::path::PathBuf::from(lock_path));
}

#[test]
fn disk_store_rejects_a_competing_coordinator_until_the_owner_drops() -> Result<(), String> {
    let path = unique_store_path("single-writer");
    let first = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    assert!(matches!(
        SqliteLifecycleStore::open(&path),
        Err(LifecycleStoreError::Conflict)
    ));
    drop(first);
    let reopened = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    drop(reopened);
    remove_store_files(&path);
    Ok(())
}

#[test]
fn disk_store_fences_a_stale_coordinator_after_lock_replacement() -> Result<(), String> {
    let path = unique_store_path("fenced-coordinator");
    let first_store = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    let mut first = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        FakeProcess::default(),
        first_store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    first
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;

    remove_coordinator_lock(&path);
    let second_store = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    let mut second = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        FakeProcess::default(),
        second_store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    second
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;

    assert_eq!(
        first.apply(super::launch_request(1)),
        Err(LifecycleError::Store(LifecycleStoreError::Conflict))
    );
    assert_eq!(first.process().starts(), 0);
    second
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(second.process().starts(), 1);

    let (_, first_store) = first.into_parts();
    let (_, second_store) = second.into_parts();
    drop(first_store);
    drop(second_store);
    remove_store_files(&path);
    Ok(())
}

#[test]
fn disk_store_releases_a_namespace_after_a_confirmed_stop() -> Result<(), String> {
    let path = unique_store_path("stop-release");
    let store = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    let process = FakeProcess::default();
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        process,
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(LifecycleRequest::stop(
            OperationId::new(2),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            super::StopMode::Force,
        ))
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(3))
        .map_err(|error| error.to_string())?;
    assert_eq!(lifecycle.process().starts(), 2);

    let (_, store) = lifecycle.into_parts();
    drop(store);
    remove_store_files(&path);
    Ok(())
}

#[test]
fn disk_store_reopens_with_durable_ownership_and_replays_without_starting_again()
-> Result<(), String> {
    let path = unique_store_path("recovery");
    let store = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(2).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        FakeProcess::default(),
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (process, store) = lifecycle.into_parts();
    drop(store);

    let store = SqliteLifecycleStore::open(&path).map_err(|error| format!("{error:?}"))?;
    let mut restarted = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(2).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        process,
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    restarted
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;
    let replay = restarted
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.operation_state(), LifecycleOperationState::Started);
    assert_eq!(restarted.process().starts(), 1);
    assert!(
        restarted
            .ownership
            .get(&InstanceId::new(7))
            .and_then(|owner| owner.process())
            .is_some()
    );
    assert_eq!(
        restarted.apply(super::launch_request(2)),
        Err(LifecycleError::InstanceBusy)
    );
    let (_, store) = restarted.into_parts();
    drop(store);
    remove_store_files(&path);
    Ok(())
}
