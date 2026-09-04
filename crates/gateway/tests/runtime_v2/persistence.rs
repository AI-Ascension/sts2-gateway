// SPDX-License-Identifier: MIT

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation,
    RuntimeV2PersistedOperation, RuntimeV2PersistedState, RuntimeV2Status,
};

use super::{FakeForwarder, FakeMode, action_request, binding};

fn state_request(correlation_id: &str, generation: u64) -> RuntimeV2Message {
    RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        correlation_id,
        "instance-1",
        "session-1",
        "lease-1",
        1,
        generation,
    )
}

#[test]
fn failed_admission_checkpoint_removes_work_before_dispatch() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;

    let result = ledger.submit_action_with_checkpoint(
        action_request("corr-admission-failure", "op-admission-failure", 1, 4),
        |_| Err(()),
    );

    assert_eq!(
        result,
        Err(sts2_gateway::RuntimeV2LedgerError::PersistenceFailed)
    );
    assert_eq!(ledger.operation_count(), 0);
    assert_eq!(fake.dispatches(), 0);
    assert_eq!(fake.applications(), 0);
    Ok(())
}

#[test]
fn failed_result_checkpoint_publishes_unknown_without_retry() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let mut checkpoints = 0;
    let request = action_request("corr-result-failure", "op-result-failure", 1, 4);

    let result = ledger
        .submit_action_with_checkpoint(request.clone(), |_| {
            checkpoints += 1;
            if checkpoints == 1 { Ok(()) } else { Err(()) }
        })
        .map_err(|error| error.to_string())?;

    assert_eq!(result.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(
        result.error_code.as_deref(),
        Some("sts2.runtime/persistence_uncertain")
    );
    assert_eq!(ledger.operation_count(), 1);
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);

    let replay = ledger
        .submit_action(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay, result);
    assert_eq!(fake.dispatches(), 1);
    Ok(())
}

#[test]
fn accepted_result_restores_as_unknown_without_dispatch_retry() -> Result<(), String> {
    let request = action_request("corr-accepted", "op-accepted", 1, 4);
    let accepted = RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        "corr-accepted",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-accepted",
        RuntimeV2Action::end_turn(),
        RuntimeV2Status::Accepted,
        Some(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            2,
            true,
            4,
        )),
        None,
        None,
        RuntimeV2MessageKind::ActionResponse,
    );
    let persisted = RuntimeV2PersistedState {
        instance_id: String::from("instance-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        observation: RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
        operations: vec![RuntimeV2PersistedOperation {
            request,
            result: Some(accepted),
        }],
    };
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .restore_state(persisted)
        .map_err(|error| error.to_string())?;

    let reconciled = ledger
        .reconcile(super::reconcile_request("corr-reconcile", "op-accepted", 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(reconciled.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(
        reconciled.error_code.as_deref(),
        Some("sts2.runtime/restart_uncertain")
    );
    assert_eq!(fake.dispatches(), 0);
    assert_eq!(fake.receipt_reads(), 1);
    Ok(())
}

#[test]
fn authoritative_state_refreshes_the_action_generation() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake)
        .map_err(|error| error.to_string())?;
    let request = state_request("corr-state", 4);
    let response = RuntimeV2Message::state_response(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 7, true, 9),
    );

    ledger
        .accept_state_response(&request, response)
        .map_err(|error| error.to_string())?;
    assert_eq!(ledger.observation().turn_index, 7);
    assert_eq!(ledger.observation().generation, 9);
    Ok(())
}

#[test]
fn settled_receipt_restores_without_mutation_retry() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake)
        .map_err(|error| error.to_string())?;
    let action = action_request("corr-action", "op-restart", 1, 4);
    let settled = ledger
        .submit_action(action)
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
    let persisted = ledger.persisted_state();

    let restarted_fake = FakeForwarder::new(FakeMode::Settle);
    let mut restarted = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(4),
        binding()?,
        restarted_fake.clone(),
    )
    .map_err(|error| error.to_string())?;
    restarted
        .restore_state(persisted)
        .map_err(|error| error.to_string())?;
    let reconciled = restarted
        .reconcile(super::reconcile_request("corr-restart", "op-restart", 5))
        .map_err(|error| error.to_string())?;
    assert_eq!(reconciled.status, Some(RuntimeV2Status::Settled));
    assert_eq!(restarted_fake.dispatches(), 0);
    assert_eq!(restarted_fake.receipt_reads(), 0);
    Ok(())
}

#[test]
fn admitted_without_result_restores_as_unknown_without_dispatch_retry() -> Result<(), String> {
    let request = action_request("corr-action", "op-inflight", 1, 4);
    let state = RuntimeV2PersistedState {
        instance_id: String::from("instance-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        observation: RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
        operations: vec![RuntimeV2PersistedOperation {
            request,
            result: None,
        }],
    };
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .restore_state(state)
        .map_err(|error| error.to_string())?;
    let reconciled = ledger
        .reconcile(super::reconcile_request("corr-reconcile", "op-inflight", 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(reconciled.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(
        reconciled.error_code.as_deref(),
        Some("sts2.runtime/restart_uncertain")
    );
    assert_eq!(fake.dispatches(), 0);
    assert_eq!(fake.receipt_reads(), 1);
    Ok(())
}

#[test]
fn persisted_state_cannot_cross_lease_identity() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake)
        .map_err(|error| error.to_string())?;
    let mut persisted = ledger.persisted_state();
    persisted.lease_id = String::from("other-lease");
    let mut restarted = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(4),
        binding()?,
        FakeForwarder::new(FakeMode::Settle),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        restarted.restore_state(persisted),
        Err(sts2_gateway::RuntimeV2LedgerError::PersistedStateMismatch)
    );
    Ok(())
}
