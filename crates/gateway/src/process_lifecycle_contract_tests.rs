// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, FakeProcess, InMemoryLifecycleStore, InstanceId, LaunchProfileId,
    LifecycleOperationState, LifecycleRequest, ProcessDescendantIdentity, ProcessFault, StopMode,
    new_lifecycle,
};
use crate::{
    LifecycleError, LifecycleOperation, LifecycleRecordStore, LifecycleStoreError, OperationId,
};

impl super::FakeProcess {
    fn set_descendants_after_stop(&mut self, descendants: Vec<ProcessDescendantIdentity>) {
        self.descendants_after_stop = descendants;
    }

    fn set_recover_fault(&mut self, fault: Option<ProcessFault>) {
        self.recover_fault = fault;
    }

    fn crash(
        &mut self,
        process: super::ProcessHandle,
        descendants: Vec<ProcessDescendantIdentity>,
    ) {
        if let Some(entry) = self.entries.get_mut(&process) {
            entry.state = super::ProcessState::Exited { code: Some(17) };
            entry.descendants = descendants;
        }
    }
}

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
fn caller_operation_ids_do_not_replace_the_server_ordered_owner() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(5))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        lifecycle
            .operation(super::InstanceId::new(7), OperationId::new(5))
            .map(|operation| operation.sequence()),
        Some(1)
    );
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
        lifecycle
            .operation(super::InstanceId::new(7), OperationId::new(4))
            .map(|operation| operation.sequence()),
        Some(2)
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
