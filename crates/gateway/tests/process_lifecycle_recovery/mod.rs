// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn service_restart_reconciles_a_persisted_process_without_relaunch() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut first = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    let started = first
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    let identity = started.process().cloned().ok_or("missing identity")?;
    let store = first.into_store();
    process.set_recover(true);
    let mut restarted = manager(process.clone(), store, lease, 1)?;
    let attached = restarted
        .apply(LifecycleRequest::attach_existing(
            sts2_gateway::OperationId::new(2),
            lease.proof(),
            AuthorityEpoch::new(1),
            identity,
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        attached.operation_state(),
        LifecycleOperationState::Attached
    );
    assert_eq!(process.starts(), 1);
    Ok(())
}

#[test]
fn capacity_is_bounded_before_a_second_launch() -> Result<(), String> {
    let (lease, second_lease) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    lifecycle
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(1),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(second_lease)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        lifecycle.apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(2),
            second_lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        )),
        Err(LifecycleError::CapacityExceeded)
    );
    assert_eq!(process.starts(), 1);
    Ok(())
}

#[test]
fn a_crashed_owned_process_is_not_reported_ready() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    let started = lifecycle
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(10),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    let identity = started.process().cloned().ok_or("missing identity")?;
    process.crash(identity.process());
    assert_eq!(
        lifecycle.reconcile(
            lease.proof(),
            AuthorityEpoch::new(1),
            started.operation_id(),
        ),
        Err(LifecycleError::Process(ProcessFault::InspectionFailed))
    );
    assert_eq!(
        lifecycle
            .operation(lease.instance_id(), started.operation_id())
            .ok_or("missing operation")?
            .state(),
        LifecycleOperationState::Failed
    );
    Ok(())
}

#[test]
fn attach_rejects_a_foreign_pid_or_birth_identity() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut lifecycle = manager(process.clone(), InMemoryLifecycleStore::new(), lease, 1)?;
    let started = lifecycle
        .apply(LifecycleRequest::launch_new(
            sts2_gateway::OperationId::new(11),
            lease.proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(1),
        ))
        .map_err(|error| error.to_string())?;
    let identity = started.process().cloned().ok_or("missing identity")?;
    let foreign = ProcessIdentity::new(
        identity.instance_id(),
        identity.process(),
        identity.pid().saturating_add(1),
        identity.birth_id(),
        identity.executable().ok_or("missing executable")?,
        identity.user_data().ok_or("missing user data")?,
    );
    assert_eq!(
        lifecycle.apply(LifecycleRequest::attach_existing(
            sts2_gateway::OperationId::new(12),
            lease.proof(),
            AuthorityEpoch::new(1),
            foreign,
        )),
        Err(LifecycleError::IdentityMismatch)
    );
    assert_eq!(process.starts(), 1);
    assert_eq!(process.stops(), 0);
    Ok(())
}

#[test]
fn launch_intent_reconciles_after_crash_before_process_creation() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let (catalog, _) = profile_catalog()?;
    let mut store = InMemoryLifecycleStore::new();
    let request = LifecycleRequest::launch_new(
        sts2_gateway::OperationId::new(8),
        lease.proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    store
        .insert(sts2_gateway::LifecycleOperation::new(
            request.operation_id(),
            request.instance_id(),
            request.lease(),
            request.authority_epoch(),
            request.action().clone(),
        ))
        .map_err(|error| format!("{error:?}"))?;
    let process = RecordingProcess::default();
    let mut lifecycle = ProcessLifecycle::new(
        ProcessLifecycleConfig::new(1),
        catalog,
        TestClock,
        process.clone(),
        store,
        DeterministicLeaseDecision,
    )
    .map_err(|error| error.to_string())?;
    lifecycle
        .bind_lease(lease)
        .map_err(|error| error.to_string())?;
    let result = lifecycle
        .reconcile(
            lease.proof(),
            AuthorityEpoch::new(1),
            request.operation_id(),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(result.operation_state(), LifecycleOperationState::Unknown);
    assert_eq!(process.starts(), 0);
    Ok(())
}

#[test]
fn launch_intent_reconciles_after_a_lost_response_without_relaunch() -> Result<(), String> {
    let (lease, _) = allocations()?;
    let process = RecordingProcess::default();
    let mut store = InMemoryLifecycleStore::new();
    store.set_fail_update_after(Some(1));
    let mut lifecycle = manager(process.clone(), store, lease, 1)?;
    let request = LifecycleRequest::launch_new(
        sts2_gateway::OperationId::new(9),
        lease.proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    assert_eq!(
        lifecycle.apply(request.clone()),
        Err(LifecycleError::Store(
            sts2_gateway::LifecycleStoreError::Database
        ))
    );
    assert_eq!(process.starts(), 1);
    let mut store = lifecycle.into_store();
    store.set_fail_update_after(None);
    process.set_recover(true);
    let mut restarted = manager(process.clone(), store, lease, 1)?;
    let reconciled = restarted
        .apply(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        reconciled.operation_state(),
        LifecycleOperationState::Started
    );
    assert_eq!(process.starts(), 1);
    Ok(())
}
