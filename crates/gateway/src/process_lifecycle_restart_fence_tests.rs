// SPDX-License-Identifier: MIT

use super::{FakeProcess, new_lifecycle};
use crate::{
    AuthorityEpoch, InMemoryLifecycleStore, InstanceId, LaunchProfileId, Lease, LifecycleError,
    LifecycleOperation, LifecycleOperationState, LifecycleOwnership, LifecycleRecordStore,
    LifecycleRequest, LifecycleStoreError, OperationId, ProcessGenerationKind, StopMode,
};

fn lease() -> Lease {
    super::lease()
}

#[test]
fn persisted_restart_recovery_rotates_epoch_before_replacing_process() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let old_identity = started
        .process()
        .cloned()
        .ok_or_else(|| String::from("initial launch omitted its process identity"))?;
    let (mut process, mut store) = lifecycle.into_parts();
    process.set_retain_after_stop(true);
    store.set_fail_update_after(Some(1));
    let mut lifecycle = new_lifecycle(process, store)?;

    let restart = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    assert_eq!(
        lifecycle.apply(restart),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    assert_eq!(lifecycle.process().starts(), 1);
    assert!(lifecycle.process().stop_modes().is_empty());

    // The first restart update is durable, but the epoch-rotation update failed.
    // Reopening simulates a crash in exactly the persisted Restarting window.
    let (process, mut store) = lifecycle.into_parts();
    let persisted = store
        .list()
        .map_err(|error| format!("{error:?}"))?
        .into_iter()
        .find(|operation| operation.operation_id() == OperationId::new(2))
        .ok_or_else(|| String::from("restart intent was not persisted"))?;
    assert_eq!(persisted.state(), LifecycleOperationState::Restarting);
    assert_eq!(persisted.authority_epoch(), AuthorityEpoch::new(1));
    assert_eq!(persisted.process(), Some(&old_identity));

    // A second fault at the recovery boundary must still leave the retained
    // process untouched until the new authority epoch is durable.
    store.set_fail_update_after(Some(0));
    let mut restarted = new_lifecycle(process, store)?;
    assert_eq!(
        restarted.reconcile(lease().proof(), AuthorityEpoch::new(1), OperationId::new(2)),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    assert_eq!(
        restarted.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(1)
    );
    assert_eq!(restarted.process().starts(), 1);
    assert!(restarted.process().stop_modes().is_empty());

    let (process, mut store) = restarted.into_parts();
    let still_unrotated = store
        .list()
        .map_err(|error| format!("{error:?}"))?
        .into_iter()
        .find(|operation| operation.operation_id() == OperationId::new(2))
        .ok_or_else(|| String::from("restart operation disappeared after failed recovery"))?;
    assert_eq!(still_unrotated.state(), LifecycleOperationState::Restarting);
    assert_eq!(still_unrotated.authority_epoch(), AuthorityEpoch::new(1));
    assert_eq!(still_unrotated.process(), Some(&old_identity));

    store.set_fail_update_after(None);
    let mut restarted = new_lifecycle(process, store)?;
    let recovered = restarted
        .reconcile(lease().proof(), AuthorityEpoch::new(1), OperationId::new(2))
        .map_err(|error| error.to_string())?;

    assert_eq!(recovered.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(
        restarted.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(2)
    );
    assert_eq!(restarted.process().starts(), 2);
    assert_eq!(restarted.process().stop_modes(), vec![StopMode::Force]);
    assert_eq!(
        restarted.apply(super::launch_request(3)),
        Err(LifecycleError::StaleAuthorityEpoch)
    );
    assert_eq!(restarted.process().starts(), 2);
    Ok(())
}

#[test]
fn already_rotated_restart_recovery_does_not_rotate_again() -> Result<(), String> {
    let mut process = FakeProcess::default();
    process.set_retain_after_stop(true);
    let mut lifecycle = new_lifecycle(process, InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| String::from("initial launch omitted its process identity"))?;
    let (process, mut store) = lifecycle.into_parts();
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    let mut operation = LifecycleOperation::new(
        request.operation_id(),
        request.instance_id(),
        request.lease(),
        request.authority_epoch(),
        request.action().clone(),
    );
    operation.set_sequence(2);
    store
        .insert(operation.clone())
        .map_err(|error| format!("{error:?}"))?;

    let mut before_restart_effect = new_lifecycle(process, store)?;
    let rotated = before_restart_effect
        .begin_restart(operation, identity)
        .map_err(|error| error.to_string())?;
    assert_eq!(rotated.authority_epoch(), AuthorityEpoch::new(2));
    let (process, store) = before_restart_effect.into_parts();

    let mut reopened = new_lifecycle(process, store)?;
    let recovered = reopened
        .reconcile(lease().proof(), AuthorityEpoch::new(2), OperationId::new(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(recovered.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(
        reopened.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(2)
    );
    assert_eq!(reopened.process().starts(), 2);
    assert_eq!(reopened.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}

#[test]
fn replacement_recovery_reuses_the_persisted_post_fence_generation_key() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let (process, mut store) = lifecycle.into_parts();
    store.set_fail_update_after(Some(3));
    let mut lifecycle = new_lifecycle(process, store)?;
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );

    assert_eq!(
        lifecycle.apply(request),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    assert_eq!(lifecycle.process().starts(), 2);
    assert_eq!(lifecycle.process().stop_modes(), vec![StopMode::Force]);
    let (process, mut store) = lifecycle.into_parts();
    store.set_fail_update_after(None);
    let persisted = store
        .list()
        .map_err(|error| format!("{error:?}"))?
        .into_iter()
        .find(|operation| operation.operation_id() == OperationId::new(2))
        .ok_or_else(|| String::from("restart operation was not retained"))?;
    assert_eq!(persisted.sequence(), 2);
    assert_eq!(persisted.request_epoch(), AuthorityEpoch::new(1));
    assert_eq!(persisted.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(persisted.state(), LifecycleOperationState::Restarting);
    assert_eq!(persisted.process(), None);

    let mut reopened = new_lifecycle(process, store)?;
    let recovered = reopened
        .reconcile(lease().proof(), AuthorityEpoch::new(2), OperationId::new(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        recovered.operation_state(),
        LifecycleOperationState::Started
    );
    assert_eq!(recovered.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(
        reopened.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(2)
    );
    assert_eq!(reopened.process().starts(), 2);
    assert_eq!(reopened.process().stop_modes(), vec![StopMode::Force]);
    assert_eq!(reopened.process().generation_starts().len(), 2);
    assert_eq!(reopened.process().generation_queries().len(), 1);
    assert_eq!(
        reopened.process().generation_starts().last(),
        reopened.process().generation_queries().last()
    );
    let generation = reopened
        .process()
        .generation_queries()
        .last()
        .ok_or_else(|| String::from("replacement generation was not recovered"))?;
    assert_eq!(generation.operation_id(), OperationId::new(2));
    assert_eq!(generation.sequence(), 2);
    assert_eq!(generation.request_epoch(), AuthorityEpoch::new(1));
    assert_eq!(generation.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(generation.profile_id(), LaunchProfileId::new(1));
    assert_eq!(generation.kind(), ProcessGenerationKind::RestartReplacement);
    Ok(())
}

#[test]
fn restart_recovery_epoch_exhaustion_has_no_process_effects() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(super::launch_request(1))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| String::from("initial launch omitted its process identity"))?;
    let (process, mut store) = lifecycle.into_parts();
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(u64::MAX),
        LaunchProfileId::new(1),
    );
    let mut operation = LifecycleOperation::new(
        request.operation_id(),
        request.instance_id(),
        request.lease(),
        request.authority_epoch(),
        request.action().clone(),
    );
    operation.set_sequence(2);
    operation.set_state(
        LifecycleOperationState::Restarting,
        Some(identity.clone()),
        None,
    );
    store
        .insert(operation)
        .map_err(|error| format!("{error:?}"))?;
    let mut reopened = new_lifecycle(process, store)?;

    assert_eq!(
        reopened.reconcile(
            lease().proof(),
            AuthorityEpoch::new(u64::MAX),
            OperationId::new(2),
        ),
        Err(LifecycleError::AuthorityExhausted)
    );
    assert_eq!(
        reopened.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(u64::MAX)
    );
    assert_eq!(reopened.process().starts(), 1);
    assert!(reopened.process().stop_modes().is_empty());
    let (_, store) = reopened.into_parts();
    let unchanged = store
        .list()
        .map_err(|error| format!("{error:?}"))?
        .into_iter()
        .find(|operation| operation.operation_id() == OperationId::new(2))
        .ok_or_else(|| String::from("restart operation disappeared at epoch exhaustion"))?;
    assert_eq!(unchanged.state(), LifecycleOperationState::Restarting);
    assert_eq!(unchanged.authority_epoch(), AuthorityEpoch::new(u64::MAX));
    assert_eq!(unchanged.process(), Some(&identity));
    Ok(())
}

#[test]
fn retained_profile_mismatch_cleanup_waits_for_restart_epoch_persistence() -> Result<(), String> {
    let mut lifecycle = new_lifecycle(FakeProcess::default(), InMemoryLifecycleStore::new())?;
    let started = lifecycle
        .apply(LifecycleRequest::launch_new(
            OperationId::new(1),
            lease().proof(),
            AuthorityEpoch::new(1),
            LaunchProfileId::new(2),
        ))
        .map_err(|error| error.to_string())?;
    let identity = started
        .process()
        .cloned()
        .ok_or_else(|| String::from("profile-2 launch omitted its process identity"))?;
    let (process, mut store) = lifecycle.into_parts();
    let request = LifecycleRequest::restart(
        OperationId::new(2),
        lease().proof(),
        AuthorityEpoch::new(1),
        LaunchProfileId::new(1),
    );
    let mut operation = LifecycleOperation::new(
        request.operation_id(),
        request.instance_id(),
        request.lease(),
        request.authority_epoch(),
        request.action().clone(),
    );
    operation.set_sequence(2);
    operation.set_state(
        LifecycleOperationState::Restarting,
        Some(identity.clone()),
        None,
    );
    store
        .insert(operation)
        .map_err(|error| format!("{error:?}"))?;
    store
        .set_ownership(LifecycleOwnership::new(
            InstanceId::new(7),
            OperationId::new(2),
            2,
            LaunchProfileId::new(1),
            Some(identity.clone()),
        ))
        .map_err(|error| format!("{error:?}"))?;
    store.set_fail_update_after(Some(0));
    let mut reopened = new_lifecycle(process, store)?;

    assert_eq!(
        reopened.reconcile(lease().proof(), AuthorityEpoch::new(1), OperationId::new(2)),
        Err(LifecycleError::Store(LifecycleStoreError::Database))
    );
    assert_eq!(
        reopened.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(1)
    );
    assert_eq!(reopened.process().starts(), 1);
    assert!(reopened.process().stop_modes().is_empty());

    let (process, mut store) = reopened.into_parts();
    store.set_fail_update_after(None);
    let mut reopened = new_lifecycle(process, store)?;
    let cleaned = reopened
        .reconcile(lease().proof(), AuthorityEpoch::new(1), OperationId::new(2))
        .map_err(|error| error.to_string())?;
    assert_eq!(cleaned.authority_epoch(), AuthorityEpoch::new(2));
    assert_eq!(cleaned.operation_state(), LifecycleOperationState::Failed);
    assert_eq!(cleaned.process(), None);
    assert_eq!(
        reopened.current_authority_epoch(InstanceId::new(7)),
        AuthorityEpoch::new(2)
    );
    assert_eq!(reopened.process().starts(), 1);
    assert_eq!(reopened.process().stop_modes(), vec![StopMode::Force]);
    Ok(())
}
