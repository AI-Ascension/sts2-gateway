// SPDX-License-Identifier: MIT

use super::{
    AuthorityEpoch, FakeProcess, InMemoryLifecycleStore, InstanceId, LifecycleRequest,
    ProcessFault, StopMode, new_lifecycle,
};
use crate::{LifecycleError, OperationId};

#[test]
fn retained_owner_snapshot_keeps_identityless_uncertain_reservation() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_start_fault(Some(ProcessFault::StopFailed));
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    let instance = InstanceId::new(7);
    assert!(lifecycle.retained_ownership(instance).is_none());
    assert_eq!(
        lifecycle.apply(super::launch_request(1)),
        Err(LifecycleError::Process(ProcessFault::StopFailed))
    );
    let retained = lifecycle
        .retained_ownership(instance)
        .ok_or("uncertain launch lost its durable capacity reservation")?;
    assert_eq!(retained.operation_id(), OperationId::new(1));
    assert_eq!(retained.profile_id(), super::LaunchProfileId::new(1));
    assert!(retained.process().is_none());
    assert_eq!(lifecycle.process().starts(), 0);
    Ok(())
}

#[test]
fn retained_owner_snapshot_is_read_only_and_clears_after_stop() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let instance = InstanceId::new(7);
    assert!(lifecycle.retained_ownership(instance).is_none());
    let launched = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let retained = lifecycle
        .retained_ownership(instance)
        .ok_or("launch lost its durable ownership")?;
    assert_eq!(retained.operation_id(), OperationId::new(1));
    assert_eq!(retained.profile_id(), super::LaunchProfileId::new(1));
    assert_eq!(retained.process(), launched.process());
    assert_eq!(lifecycle.process().starts(), 1);
    lifecycle
        .apply(LifecycleRequest::stop(
            OperationId::new(2),
            super::lease().proof(),
            AuthorityEpoch::new(1),
            StopMode::Force,
        ))
        .map_err(|error| error.to_string())?;
    assert!(lifecycle.retained_ownership(instance).is_none());
    assert_eq!(lifecycle.process().starts(), 1);
    Ok(())
}
