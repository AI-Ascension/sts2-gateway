// SPDX-License-Identifier: MIT

use super::{FakeClock, FakeProcess, new_lifecycle, profiles};
use crate::{
    AuthorityEpoch, DeterministicLeaseDecision, ExecutableIdentity, InMemoryLifecycleStore,
    InstanceId, LaunchProfileId, Lease, LifecycleError, LifecycleOperation,
    LifecycleOperationState, LifecycleRecordStore, LifecycleRequest, LifecycleState,
    LifecycleStoreError, OperationId, ProcessDescendantIdentity, ProcessFault, ProcessHandle,
    ProcessIdentity, ProcessLifecycle, ProcessLifecycleConfig, ProcessPolicy, SqliteLifecycleStore,
    StopMode, UserDataConfig,
};

fn lease() -> Lease {
    super::lease()
}

#[test]
fn stop_failure_is_blocked_then_explicit_cleanup_retry_can_settle() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (mut process, store) = lifecycle.into_parts();
    process.set_stop_fault(Some(ProcessFault::StopTimedOut));
    process.set_retain_after_stop(true);
    let mut lifecycle = new_lifecycle(process, store)?;
    let stop = LifecycleRequest::stop(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        StopMode::Force,
    );
    assert_eq!(
        lifecycle.apply(stop),
        Err(LifecycleError::Process(ProcessFault::StopTimedOut))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Blocked)
    );
    let (mut process, store) = lifecycle.into_parts();
    process.set_stop_fault(None);
    let mut lifecycle = new_lifecycle(process, store)?;
    let retry = LifecycleRequest::stop(
        OperationId::new(3),
        lease().proof(),
        AuthorityEpoch::new(1),
        StopMode::Force,
    );
    let stopped = lifecycle.apply(retry).map_err(|error| error.to_string())?;
    assert_eq!(stopped.state(), LifecycleState::Stopped);
    assert_eq!(stopped.operation_state(), LifecycleOperationState::Stopped);
    assert!(started.process().is_some());
    Ok(())
}

#[test]
fn restart_rotates_authority_and_rejects_old_epoch_requests() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_retain_after_stop(true);
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let restart = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    let response = lifecycle
        .apply(restart)
        .map_err(|error| error.to_string())?;
    assert_eq!(response.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(lifecycle.process().starts(), 2);
    assert_eq!(lifecycle.process().stop_modes(), vec![StopMode::Force]);
    let stale = LifecycleRequest::launch_new(
        OperationId::new(3),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    assert_eq!(
        lifecycle.apply(stale),
        Err(LifecycleError::StaleAuthorityEpoch)
    );
    Ok(())
}

#[test]
fn persisted_start_intent_reconciles_without_restarting_the_process() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_recover_enabled(true);
    let mut store = InMemoryLifecycleStore::new();
    store.set_fail_update_after(Some(1));
    let mut lifecycle = new_lifecycle(process, store)?;
    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    let (process, mut store) = lifecycle.into_parts();
    store.set_fail_update_after(None);
    let mut restarted = new_lifecycle(process, store)?;
    let reconciled = restarted
        .reconcile(lease().proof(), AuthorityEpoch::new(1), OperationId::new(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        reconciled.operation_state(),
        LifecycleOperationState::Started
    );
    assert_eq!(reconciled.state(), LifecycleState::Starting);
    assert_eq!(restarted.process().starts(), 1);
    Ok(())
}

#[test]
fn pending_stop_intent_reconciles_a_running_process() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (process, mut store) = lifecycle.into_parts();
    let request = LifecycleRequest::stop(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        StopMode::Force,
    );
    store
        .insert(LifecycleOperation::new(
            request.operation_id(),
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
            request.action().clone(),
        ))
        .map_err(|error| format!("{error:?}"))?;
    let mut restarted = new_lifecycle(process, store)?;
    let response = restarted
        .reconcile(
            lease().proof(),
            AuthorityEpoch::new(1),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(response.state(), LifecycleState::Stopped);
    assert_eq!(restarted.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}

#[test]
fn attach_to_an_unowned_process_is_rejected_without_a_port_call() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let identity = ProcessIdentity::new(
        InstanceId::new(7),
        ProcessHandle::new(900),
        901,
        902,
        ExecutableIdentity::new(101, 202, 404),
        UserDataConfig::new(303),
    );
    let request = LifecycleRequest::attach_existing(
        OperationId::new(1),
        lease().proof(),
        AuthorityEpoch::new(1),
        identity,
    );
    assert_eq!(lifecycle.apply(request), Err(LifecycleError::UnownedAttach));
    assert_eq!(lifecycle.process().starts(), 0);
    assert!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(1))
            .is_none()
    );
    Ok(())
}

#[test]
fn pending_restart_intent_stops_before_starting_a_replacement() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (mut process, mut store) = lifecycle.into_parts();
    process.set_retain_after_stop(true);
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    store
        .insert(LifecycleOperation::new(
            request.operation_id(),
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
            request.action().clone(),
        ))
        .map_err(|error| format!("{error:?}"))?;
    let mut restarted = new_lifecycle(process, store)?;
    let response = restarted
        .reconcile(
            lease().proof(),
            AuthorityEpoch::new(1),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(response.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(response.operation_state(), LifecycleOperationState::Started);
    assert_eq!(restarted.process().starts(), 2);
    assert_eq!(restarted.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}

#[test]
fn profile_policy_rejects_zero_descendants_and_unbounded_timeouts() -> Result<(), String> {
    assert_eq!(
        ProcessPolicy::try_new(0, 1_000, 1_000),
        Err(crate::LaunchProfileError::InvalidPolicy)
    );
    assert_eq!(
        ProcessPolicy::try_new(1, 0, 1_000),
        Err(crate::LaunchProfileError::InvalidPolicy)
    );
    assert_eq!(
        ProcessPolicy::try_new(1, 1_000, 60_001),
        Err(crate::LaunchProfileError::InvalidPolicy)
    );
    Ok(())
}

#[test]
fn restart_without_recovery_observation_stays_unknown_without_relaunch() -> Result<(), String> {
    let request = LifecycleRequest::restart(
        OperationId::new(4),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    let mut operation = LifecycleOperation::new(
        request.operation_id(),
        request.instance_id(),
        request.lease(),
        AuthorityEpoch::new(2),
        request.action().clone(),
    );
    operation.set_state(LifecycleOperationState::Restarting, None, None);
    let mut store = InMemoryLifecycleStore::new();
    store
        .insert(operation)
        .map_err(|error| format!("{error:?}"))?;

    let process = FakeProcess::default();
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        process.clone(),
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(lease())
        .map_err(|error| error.to_string())?;

    let result = lifecycle
        .reconcile(
            lease().proof(),
            AuthorityEpoch::new(2),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(result.state(), LifecycleState::Unknown);
    assert_eq!(result.operation_state(), LifecycleOperationState::Unknown);
    assert_eq!(process.starts(), 0);
    let replay = lifecycle
        .reconcile(
            lease().proof(),
            AuthorityEpoch::new(2),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.state(), LifecycleState::Unknown);
    assert_eq!(process.starts(), 0);
    Ok(())
}

#[test]
fn sqlite_records_round_trip_and_replay_without_a_second_start() -> Result<(), String> {
    let store = SqliteLifecycleStore::open_in_memory().map_err(|error| format!("{error:?}"))?;
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
        .bind_lease(lease())
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (process, store) = lifecycle.into_parts();
    let mut restarted = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(2).map_err(|error| format!("{error:?}"))?,
        profiles()?,
        FakeClock::default(),
        process,
        store,
        super::DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    restarted
        .bind_lease(lease())
        .map_err(|error| error.to_string())?;
    let replay = restarted
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.state(), LifecycleState::Starting);
    assert_eq!(restarted.process().starts(), 1);
    Ok(())
}

#[test]
fn lifecycle_request_json_is_closed_to_unknown_members() -> Result<(), String> {
    let request = super::launch_request(1);
    let bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    let decoded: LifecycleRequest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    assert_eq!(decoded, request);
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    value["unexpected"] = serde_json::Value::Bool(true);
    let malformed = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    assert!(serde_json::from_slice::<LifecycleRequest>(&malformed).is_err());

    let mut nested =
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())?;
    nested["lease"]["unexpected"] = serde_json::Value::Bool(true);
    let malformed_lease = serde_json::to_vec(&nested).map_err(|error| error.to_string())?;
    assert!(serde_json::from_slice::<LifecycleRequest>(&malformed_lease).is_err());

    let mut action =
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|error| error.to_string())?;
    let action_variant = action["action"]
        .as_object_mut()
        .and_then(|variants| variants.values_mut().next())
        .ok_or_else(|| "missing action variant".to_owned())?;
    action_variant["unexpected"] = serde_json::Value::Bool(true);
    let malformed_action = serde_json::to_vec(&action).map_err(|error| error.to_string())?;
    assert!(serde_json::from_slice::<LifecycleRequest>(&malformed_action).is_err());
    Ok(())
}

#[test]
fn foreign_descendant_prevents_a_stop_before_the_port_stop_call() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| "missing process".to_owned())?;
    let descendant = ProcessDescendantIdentity::new(
        identity.instance_id(),
        700,
        701,
        ExecutableIdentity::new(999, 998, 997),
        UserDataConfig::new(996),
    );
    lifecycle
        .process_mut()
        .set_descendants(identity.process(), vec![descendant]);
    let stop = LifecycleRequest::stop(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        StopMode::Force,
    );
    assert_eq!(
        lifecycle.apply(stop),
        Err(LifecycleError::ForeignDescendant)
    );
    assert_eq!(lifecycle.process().stop_modes(), Vec::<StopMode>::new());
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Blocked)
    );
    Ok(())
}
