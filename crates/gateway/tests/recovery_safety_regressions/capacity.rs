// SPDX-License-Identifier: MIT

//! Incarnation-scoped recovery operation capacity regressions.

use sts2_gateway::{
    RecoveryIntentResult, RecoveryLeaseRequest, RecoveryOperationState, RecoveryStoreError,
    RecoveryUncertaintyReason,
};

use super::support::*;

fn assert_unresolved_state_blocks_new_intent(
    artifacts: &mut TempArtifacts,
    label: &str,
    state: RecoveryOperationState,
) -> Result<(), RecoveryStoreError> {
    let path = artifacts.path(label);
    let (mut store, lease, _) = ready_store(&path)?;
    let proof = lease.proof();
    let first_id = "00000000-0000-4000-8000-000000000010";
    match store.record_intent(&proof, intent(&lease, first_id)?)? {
        RecoveryIntentResult::Created(_) => {}
        RecoveryIntentResult::Duplicate(_) => return Err(RecoveryStoreError::OperationConflict),
    }
    if state != RecoveryOperationState::IntentRecorded {
        store.mark_dispatched(&proof, INSTANCE, first_id, 1_004)?;
    }
    match state {
        RecoveryOperationState::Accepted => {
            store.record_outcome(
                INSTANCE,
                first_id,
                state,
                Some(202),
                None,
                None,
                None,
                1_005,
            )?;
        }
        RecoveryOperationState::Unknown => {
            store.record_outcome(
                INSTANCE,
                first_id,
                state,
                None,
                None,
                None,
                Some(RecoveryUncertaintyReason::Timeout),
                1_005,
            )?;
        }
        RecoveryOperationState::IntentRecorded | RecoveryOperationState::MayHaveBeenDispatched => {}
        _ => return Err(RecoveryStoreError::InvalidTransition),
    }
    assert_eq!(
        store.record_intent(
            &proof,
            intent(&lease, "00000000-0000-4000-8000-000000000011")?
        ),
        Err(RecoveryStoreError::CapacityExceeded),
        "an unresolved {state:?} operation must reserve its incarnation"
    );
    Ok(())
}

#[test]
fn one_unresolved_operation_is_allowed_per_incarnation() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    for (label, state) in [
        ("intent-recorded", RecoveryOperationState::IntentRecorded),
        (
            "may-have-been-dispatched",
            RecoveryOperationState::MayHaveBeenDispatched,
        ),
        ("accepted", RecoveryOperationState::Accepted),
        ("unknown", RecoveryOperationState::Unknown),
    ] {
        assert_unresolved_state_blocks_new_intent(&mut artifacts, label, state)?;
    }
    Ok(())
}

#[test]
fn settled_operation_releases_incarnation_capacity() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("settled-releases-incarnation");
    let (mut store, lease, fence) = ready_store(&path)?;
    let first_id = "00000000-0000-4000-8000-000000000012";
    created_operation(&mut store, &lease, first_id)?;
    let settled = store.record_outcome(
        INSTANCE,
        first_id,
        RecoveryOperationState::Settled,
        Some(200),
        None,
        Some(witness(&lease, &fence, first_id, &fence.host_fence_id)?),
        None,
        1_005,
    )?;
    assert_eq!(settled.state, RecoveryOperationState::Settled);
    assert!(matches!(
        store.record_intent(
            &lease.proof(),
            intent(&lease, "00000000-0000-4000-8000-000000000013")?
        )?,
        RecoveryIntentResult::Created(_)
    ));
    Ok(())
}

#[test]
fn settled_operations_do_not_consume_unresolved_capacity() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("settled-over-unresolved-capacity");
    let (mut store, lease, fence) = ready_store(&path)?;
    for index in 0..65_u32 {
        let operation_id = format!("00000000-0000-4000-8000-00000000{index:04}");
        created_operation(&mut store, &lease, &operation_id)?;
        store.record_outcome(
            INSTANCE,
            &operation_id,
            RecoveryOperationState::Settled,
            Some(200),
            None,
            Some(witness(
                &lease,
                &fence,
                &operation_id,
                &fence.host_fence_id,
            )?),
            None,
            1_005,
        )?;
    }
    assert_eq!(store.unresolved_count()?, 0);
    assert_eq!(store.retained_count()?, 65);
    assert!(matches!(
        store.record_intent(
            &lease.proof(),
            intent(&lease, "00000000-0000-4000-8000-000000000180")?
        )?,
        RecoveryIntentResult::Created(_)
    ));
    Ok(())
}

#[test]
fn new_incarnation_has_independent_unresolved_capacity() -> Result<(), RecoveryStoreError> {
    let mut artifacts = TempArtifacts::new();
    let path = artifacts.path("independent-incarnation-capacity");
    let (mut store, old_lease, _) = ready_store(&path)?;
    created_operation(
        &mut store,
        &old_lease,
        "00000000-0000-4000-8000-000000000014",
    )?;
    store.record_outcome(
        INSTANCE,
        "00000000-0000-4000-8000-000000000014",
        RecoveryOperationState::Unknown,
        None,
        None,
        None,
        Some(RecoveryUncertaintyReason::Timeout),
        1_005,
    )?;

    let boot = store.start_boot(DEPLOYMENT, INSTANCE, release(DIGEST)?, 2_000)?;
    let fence = store.complete_host_fence(&boot, 2_001)?;
    let lease = store.acquire_lease(RecoveryLeaseRequest {
        deployment_id: DEPLOYMENT.to_owned(),
        instance_id: INSTANCE.to_owned(),
        instance_incarnation: boot.instance_incarnation.clone(),
        boot_id: boot.boot_id.clone(),
        authority_generation: boot.authority_generation,
        host_fence_id: fence.host_fence_id.clone(),
        host_fence_generation: fence.fence_generation,
        caller_id: "caller-2".to_owned(),
        session_id: "session-2".to_owned(),
        now_millis: 2_002,
        ttl_seconds: 30,
        renewal_interval_seconds: 10,
    })?;
    assert!(matches!(
        store.record_intent(
            &lease.proof(),
            intent(&lease, "00000000-0000-4000-8000-000000000015")?
        )?,
        RecoveryIntentResult::Created(_)
    ));
    assert_eq!(
        store.record_intent(
            &old_lease.proof(),
            intent(&old_lease, "00000000-0000-4000-8000-000000000016")?
        ),
        Err(RecoveryStoreError::LeaseRevoked)
    );
    Ok(())
}
