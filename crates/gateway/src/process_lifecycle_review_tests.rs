// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, DeterministicLeaseDecision, FakeClock, FakeProcess, InMemoryLifecycleStore,
    InstanceId, LifecycleOperationState, LifecycleRequest, ProcessFault, ProcessLifecycle,
    ProcessLifecycleConfig, StopMode, new_lifecycle, profiles,
};
use crate::{ApprovedLaunchProfileAdapter, LifecycleError, LifecycleFailure, OperationId};

#[test]
fn launch_failure_is_retained_without_leaking_capacity() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_start_fault(Some(ProcessFault::StartRejected));
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::Process(ProcessFault::StartRejected))
    );
    let operation = lifecycle
        .operation(InstanceId::new(7), OperationId::new(1))
        .ok_or_else(|| "missing retained operation".to_owned())?;
    assert_eq!(operation.state(), LifecycleOperationState::Failed);
    assert_eq!(
        operation.failure(),
        Some(LifecycleFailure::Process(ProcessFault::StartRejected))
    );
    lifecycle.process_mut().set_start_fault(None);
    lifecycle
        .apply(super::launch_request(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(lifecycle.process().starts(), 1);
    Ok(())
}

#[test]
fn ambiguous_launch_failure_keeps_a_capacity_reservation() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_start_fault(Some(ProcessFault::StopFailed));
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::Process(ProcessFault::StopFailed))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(1))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Unknown)
    );
    assert_eq!(
        lifecycle.apply(super::launch_request(2)),
        Err(LifecycleError::InstanceBusy)
    );
    assert_eq!(lifecycle.process().starts(), 0);
    Ok(())
}

#[test]
fn adapter_cleanup_failure_becomes_unknown_with_a_durable_reservation() -> Result<(), String> {
    let profiles = profiles()?;
    let mut process = FakeProcess::default();
    process.set_wrong_image(true);
    process.set_stop_fault(Some(ProcessFault::StopFailed));
    let adapter = ApprovedLaunchProfileAdapter::new(profiles.clone(), process);
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles,
        FakeClock::default(),
        adapter,
        InMemoryLifecycleStore::new(),
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;

    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::Process(ProcessFault::StopFailed))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(1))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Unknown)
    );
    assert_eq!(
        lifecycle
            .ownership
            .get(&InstanceId::new(7))
            .map(|owner| owner.process()),
        Some(Some(
            lifecycle
                .operation(InstanceId::new(7), OperationId::new(1))
                .and_then(|operation| operation.process())
                .ok_or_else(|| "missing retained launch identity".to_owned())?
        ))
    );
    lifecycle
        .bind_lease(super::lease_for(8))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        lifecycle.apply(super::launch_request_for(2, 8, 1)),
        Err(LifecycleError::CapacityExceeded)
    );
    assert_eq!(lifecycle.process().process().starts(), 1);

    lifecycle.process_mut().process_mut().set_stop_fault(None);
    let cleaned = lifecycle
        .reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(1),
            OperationId::new(1),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(cleaned.operation_state(), LifecycleOperationState::Failed);
    assert!(!lifecycle.ownership.contains_key(&InstanceId::new(7)));

    lifecycle.process_mut().process_mut().set_wrong_image(false);
    lifecycle
        .apply(super::launch_request(3))
        .map_err(|error| error.to_string())?;
    assert_eq!(lifecycle.process().process().starts(), 2);
    Ok(())
}

#[test]
fn restart_cleanup_reconciles_a_retained_profile_mismatch() -> Result<(), String> {
    let profiles = profiles()?;
    let mut process = FakeProcess::default();
    process.set_stop_fault_after_first(Some(ProcessFault::StopFailed));
    let adapter = ApprovedLaunchProfileAdapter::new(profiles.clone(), process);
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::try_new(1).map_err(|error| format!("{error:?}"))?,
        profiles,
        FakeClock::default(),
        adapter,
        InMemoryLifecycleStore::new(),
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(super::lease())
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    lifecycle.process_mut().process_mut().set_wrong_image(true);

    assert_eq!(
        lifecycle.apply(LifecycleRequest::restart(
            OperationId::new(2),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            super::LaunchProfileId::new(1),
        )),
        Err(LifecycleError::Process(ProcessFault::StopFailed))
    );
    let retained = lifecycle
        .operation(InstanceId::new(7), OperationId::new(2))
        .ok_or_else(|| "missing retained restart operation".to_owned())?
        .process()
        .cloned()
        .ok_or_else(|| "missing retained replacement identity".to_owned())?;
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Unknown)
    );
    assert_eq!(
        lifecycle
            .ownership
            .get(&InstanceId::new(7))
            .and_then(|owner| owner.process()),
        Some(&retained)
    );

    lifecycle
        .process_mut()
        .process_mut()
        .set_stop_fault_after_first(None);
    let cleaned = lifecycle
        .reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(2),
            OperationId::new(2),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(cleaned.operation_state(), LifecycleOperationState::Failed);
    assert_eq!(cleaned.process(), None);
    assert!(!lifecycle.ownership.contains_key(&InstanceId::new(7)));
    assert_eq!(
        lifecycle.process().process().stop_modes(),
        vec![StopMode::Force, StopMode::Force]
    );
    Ok(())
}

#[test]
fn profile_namespace_cannot_be_shared_across_instances() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .bind_lease(super::lease_for(8))
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;

    assert_eq!(
        lifecycle.apply(super::launch_request_for(2, 8, 1)),
        Err(LifecycleError::UserDataNamespaceBusy)
    );
    assert_eq!(
        lifecycle.apply(super::launch_request_for(3, 8, 2)),
        Err(LifecycleError::UserDataNamespaceBusy)
    );
    assert_eq!(lifecycle.process().starts(), 1);
    assert!(
        lifecycle
            .operation(InstanceId::new(8), OperationId::new(2))
            .is_none()
    );
    Ok(())
}

#[test]
fn wrong_identity_is_cleaned_and_never_becomes_owned() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_wrong_identity(true);
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::IdentityMismatch)
    );
    let operation = lifecycle
        .operation(InstanceId::new(7), OperationId::new(1))
        .ok_or_else(|| "missing retained operation".to_owned())?;
    assert_eq!(operation.state(), LifecycleOperationState::Rejected);
    assert_eq!(lifecycle.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}

#[test]
fn record_budget_rejects_before_a_new_process_effect() -> Result<(), String> {
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::new_with_record_budget(1, 2),
        profiles()?,
        FakeClock::default(),
        FakeProcess::default(),
        InMemoryLifecycleStore::new(),
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
            StopMode::Force,
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        lifecycle.apply(super::launch_request(3)),
        Err(LifecycleError::CapacityExceeded)
    );
    assert_eq!(lifecycle.process().starts(), 1);
    assert_eq!(lifecycle.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}

#[test]
fn stale_operation_replay_cannot_replace_a_newer_owner() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let first = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let first_identity = first.process().cloned().ok_or("missing first process")?;
    lifecycle
        .apply(LifecycleRequest::attach_existing(
            OperationId::new(2),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            first_identity.clone(),
        ))
        .map_err(|error| error.to_string())?;
    let replay = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.process(), Some(&first_identity));
    assert_eq!(
        lifecycle.owned.get(&InstanceId::new(7)),
        Some(&first_identity)
    );
    assert_eq!(
        lifecycle
            .ownership
            .get(&InstanceId::new(7))
            .map(|owner| owner.operation_id()),
        Some(OperationId::new(2))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Attached)
    );
    Ok(())
}

#[test]
fn stale_blocked_restart_is_fenced_after_stop_and_namespace_is_released() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    lifecycle
        .process_mut()
        .set_identity_fault(Some(ProcessFault::InspectionFailed));

    assert_eq!(
        lifecycle.apply(LifecycleRequest::restart(
            OperationId::new(2),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            super::LaunchProfileId::new(1),
        )),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Blocked)
    );

    lifecycle.process_mut().set_identity_fault(None);
    lifecycle.process_mut().set_retain_after_stop(true);
    let stopped = lifecycle
        .apply(LifecycleRequest::stop(
            OperationId::new(3),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            StopMode::Force,
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(stopped.operation_state(), LifecycleOperationState::Stopped);
    assert!(!lifecycle.ownership.contains_key(&InstanceId::new(7)));
    let starts_before = lifecycle.process().starts();

    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(1),
            OperationId::new(2),
        ),
        Err(LifecycleError::InstanceBusy)
    );
    assert_eq!(lifecycle.process().starts(), starts_before);
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Rejected)
    );

    lifecycle
        .bind_lease(super::lease_for(8))
        .map_err(|error| error.to_string())?;
    lifecycle
        .apply(super::launch_request_for(4, 8, 1))
        .map_err(|error| error.to_string())?;
    assert_eq!(lifecycle.process().starts(), starts_before + 1);
    Ok(())
}
