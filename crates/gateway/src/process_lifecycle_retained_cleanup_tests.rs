// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, DeterministicLeaseDecision, ExecutableIdentity, FakeClock, FakeProcess,
    InMemoryLifecycleStore, InstanceId, LifecycleOperationState, LifecycleRequest,
    ProcessDescendantIdentity, ProcessFault, ProcessLifecycle, ProcessLifecycleConfig, StopMode,
    UserDataConfig, profiles,
};
use crate::{ApprovedLaunchProfileAdapter, LifecycleError, OperationId};

#[test]
fn retained_cleanup_validates_foreign_descendants_before_force_stop() -> Result<(), String> {
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
    lifecycle
        .process_mut()
        .process_mut()
        .set_stop_fault_after_first(None);
    lifecycle.process_mut().process_mut().set_descendants(
        retained.process(),
        vec![ProcessDescendantIdentity::new(
            InstanceId::new(7),
            700,
            701,
            ExecutableIdentity::new(999, 998, 997),
            UserDataConfig::new(996),
        )],
    );

    assert_eq!(
        lifecycle.reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(2),
            OperationId::new(2),
        ),
        Err(LifecycleError::ForeignDescendant)
    );
    assert_eq!(
        lifecycle.process().process().stop_modes(),
        vec![StopMode::Force]
    );
    assert_eq!(
        lifecycle
            .operation(InstanceId::new(7), OperationId::new(2))
            .map(|operation| operation.state()),
        Some(LifecycleOperationState::Blocked)
    );

    lifecycle
        .process_mut()
        .process_mut()
        .set_descendants(retained.process(), Vec::new());
    let cleaned = lifecycle
        .reconcile(
            super::lease().proof(),
            AuthorityEpoch::new(2),
            OperationId::new(2),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(cleaned.operation_state(), LifecycleOperationState::Failed);
    assert_eq!(
        lifecycle.process().process().stop_modes(),
        vec![StopMode::Force, StopMode::Force]
    );
    Ok(())
}
