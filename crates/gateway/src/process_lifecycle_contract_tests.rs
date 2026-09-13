// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, DeterministicLeaseDecision, FakeClock, FakeProcess, InMemoryLifecycleStore,
    InstanceId, LaunchProfileId, LifecycleOperationState, LifecycleRequest,
    ProcessDescendantIdentity, ProcessFault, ProcessLifecycle, ProcessLifecycleConfig, StopMode,
    new_lifecycle, profiles,
};
use crate::{
    LifecycleError, LifecycleFailure, LifecycleOperation, LifecycleRecordStore, LifecycleState,
    LifecycleStoreError, OperationId,
};

#[test]
fn inspection_failure_keeps_identity_reserved_until_cleanup_is_observed() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| "missing process identity".to_owned())?;
    lifecycle
        .process_mut()
        .set_identity_fault(Some(ProcessFault::InspectionFailed));

    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            super::AuthorityEpoch::new(1),
            OperationId::new(1),
        ),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    let operation = lifecycle
        .operation(super::InstanceId::new(7), OperationId::new(1))
        .ok_or_else(|| "missing retained operation".to_owned())?;
    assert_eq!(operation.state(), LifecycleOperationState::Unknown);
    assert_eq!(operation.process(), Some(&identity));
    assert_eq!(
        lifecycle.owned.get(&super::InstanceId::new(7)),
        Some(&identity)
    );
    lifecycle.process_mut().set_identity_fault(None);
    let recovered = lifecycle
        .reconcile(
            super::lease().proof(),
            super::AuthorityEpoch::new(1),
            OperationId::new(1),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(
        recovered.operation_state(),
        LifecycleOperationState::Started
    );
    Ok(())
}

#[test]
fn crashed_process_with_surviving_descendants_stays_unknown_and_owned() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| "missing process identity".to_owned())?;
    let profile = super::profile(1, 404)?;
    lifecycle.process_mut().crash(
        identity.process(),
        vec![super::ProcessDescendantIdentity::new(
            identity.instance_id(),
            900,
            901,
            profile.executable(),
            profile.user_data(),
        )],
    );

    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            super::AuthorityEpoch::new(1),
            OperationId::new(1),
        ),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    let operation = lifecycle
        .operation(super::InstanceId::new(7), OperationId::new(1))
        .ok_or_else(|| "missing retained operation".to_owned())?;
    assert_eq!(operation.state(), LifecycleOperationState::Unknown);
    assert_eq!(operation.process(), Some(&identity));
    assert_eq!(
        lifecycle.owned.get(&super::InstanceId::new(7)),
        Some(&identity)
    );
    Ok(())
}

#[test]
fn failed_start_cleanup_retains_identity_when_descendants_survive() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_wrong_identity(true);
    process.set_retain_after_stop(true);
    let profile = super::profile(1, 404)?;
    process.set_descendants_after_stop(vec![super::ProcessDescendantIdentity::new(
        super::InstanceId::new(99),
        800,
        801,
        profile.executable(),
        profile.user_data(),
    )]);
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;

    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::ForeignDescendant)
    );
    let operation = lifecycle
        .operation(super::InstanceId::new(7), OperationId::new(1))
        .ok_or_else(|| "missing retained operation".to_owned())?;
    assert_eq!(operation.state(), LifecycleOperationState::Blocked);
    assert!(operation.process().is_some());
    assert_eq!(lifecycle.process().stop_modes(), vec![StopMode::Force]);
    assert!(lifecycle.owned.contains_key(&super::InstanceId::new(7)));
    Ok(())
}

#[test]
fn recovery_error_after_creation_keeps_the_instance_reserved() -> Result<(), String> {
    let process = FakeProcess::default();
    let mut store = InMemoryLifecycleStore::new();
    store.set_fail_update_after(Some(1));
    let mut lifecycle = new_lifecycle(process, store)?;
    let request = super::launch_request(1);
    assert_eq!(
        lifecycle.apply(request.clone()),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    assert_eq!(lifecycle.process().starts(), 1);
    let (mut process, mut store) = lifecycle.into_parts();
    store.set_fail_update_after(None);
    process.set_recover_fault(Some(ProcessFault::Unavailable));
    let mut restarted = new_lifecycle(process, store)?;
    let recovered = restarted
        .apply(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        recovered.operation_state(),
        LifecycleOperationState::Unknown
    );
    assert_eq!(
        restarted.apply(super::launch_request(2)),
        Err(LifecycleError::InstanceBusy)
    );
    assert_eq!(restarted.process().starts(), 1);
    Ok(())
}

#[test]
fn rejected_restart_does_not_replace_the_authoritative_owner() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let rejected = LifecycleRequest::restart(
        OperationId::new(2),
        super::lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(2),
    );
    assert_eq!(
        lifecycle.apply(rejected),
        Err(LifecycleError::IdentityMismatch)
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Rejected)
    );
    assert_eq!(
        lifecycle.apply(super::launch_request(3)),
        Err(LifecycleError::InstanceBusy)
    );
    assert_eq!(lifecycle.process().starts(), 1);
    Ok(())
}

#[test]
fn crashed_parent_with_surviving_descendant_stays_reserved_until_gone() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started.process().cloned().ok_or("missing identity")?;
    let profile = super::profile(1, 404)?;
    let descendant = ProcessDescendantIdentity::new(
        InstanceId::new(7),
        800,
        801,
        profile.executable(),
        profile.user_data(),
    );
    lifecycle
        .process_mut()
        .crash(identity.process(), vec![descendant]);
    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(1),
            OperationId::new(1),
        ),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(1))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Unknown)
    );
    lifecycle
        .process_mut()
        .set_descendants(identity.process(), vec![]);
    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(1),
            OperationId::new(1),
        ),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(1))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Failed)
    );
    lifecycle
        .apply(super::launch_request(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(lifecycle.process().starts(), 2);
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
fn caller_operation_ids_do_not_replace_the_server_ordered_owner() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(5))
        .map_err(|error| error.to_string())?;
    let request = LifecycleRequest::restart(
        OperationId::new(4),
        super::lease().proof(),
        super::AuthorityEpoch::new(1),
        super::LaunchProfileId::new(2),
    );
    assert_eq!(
        lifecycle.apply(request),
        Err(LifecycleError::IdentityMismatch)
    );
    assert!(
        lifecycle
            .operation(super::InstanceId::new(7), OperationId::new(4))
            .is_some_and(|operation| operation.state() == LifecycleOperationState::Rejected)
    );
    assert_eq!(
        lifecycle.apply(super::launch_request(3)),
        Err(LifecycleError::InstanceBusy)
    );
    assert_eq!(lifecycle.process().starts(), 1);
    Ok(())
}

#[test]
fn unknown_restart_retries_read_only_recovery_until_identity_is_found() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (process, mut store) = lifecycle.into_parts();
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        super::lease().proof(),
        super::AuthorityEpoch::new(1),
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
    store
        .insert(operation)
        .map_err(|error| format!("{error:?}"))?;
    let mut restarted = new_lifecycle(process, store)?;
    let first = restarted
        .reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(2),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(first.operation_state(), LifecycleOperationState::Unknown);
    assert_eq!(restarted.process().starts(), 1);
    restarted.process_mut().set_recover_enabled(true);
    let second = restarted
        .reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(2),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(second.operation_state(), LifecycleOperationState::Started);
    assert_eq!(second.process(), started.process());
    assert_eq!(restarted.process().starts(), 1);
    Ok(())
}

#[test]
fn recovery_inspection_fault_retains_the_intent_for_a_later_reconcile() -> Result<(), String> {
    let request = super::launch_request(8);
    let mut store = InMemoryLifecycleStore::new();
    store
        .insert(LifecycleOperation::new(
            request.operation_id(),
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
            request.action().clone(),
        ))
        .map_err(|error| format!("{error:?}"))?;
    let mut process = FakeProcess::default();
    process.set_recover_enabled(true);
    process.set_recover_fault(Some(ProcessFault::Unavailable));
    let mut lifecycle = new_lifecycle(process, store)?;

    let response = lifecycle
        .reconcile(
            super::lease().proof(),
            super::AuthorityEpoch::new(1),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(response.state(), LifecycleState::Unknown);
    assert_eq!(response.operation_state(), LifecycleOperationState::Unknown);
    assert_eq!(
        response.failure(),
        Some(LifecycleFailure::Process(ProcessFault::Unavailable))
    );
    Ok(())
}
