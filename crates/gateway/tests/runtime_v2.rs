// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2EffectWitness,
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation,
    RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault, verify_runtime_v2_artifact,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FakeMode {
    Settle,
    AcceptedThenReceipt,
    DisconnectAfterWrite,
    DisconnectWithoutReceipt,
}

struct FakeState {
    mode: FakeMode,
    dispatches: usize,
    receipt_reads: usize,
    applications: usize,
    retained_receipt: Option<RuntimeV2Message>,
}

#[derive(Clone)]
struct FakeForwarder(Rc<RefCell<FakeState>>);

impl FakeForwarder {
    fn new(mode: FakeMode) -> Self {
        Self(Rc::new(RefCell::new(FakeState {
            mode,
            dispatches: 0,
            receipt_reads: 0,
            applications: 0,
            retained_receipt: None,
        })))
    }

    fn dispatches(&self) -> usize {
        self.0.borrow().dispatches
    }

    fn receipt_reads(&self) -> usize {
        self.0.borrow().receipt_reads
    }

    fn applications(&self) -> usize {
        self.0.borrow().applications
    }
}

impl RuntimeV2ForwardingPort for FakeForwarder {
    fn forward_runtime_v2(
        &mut self,
        request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        let mut state = self.0.borrow_mut();
        state.dispatches += 1;
        let mode = state.mode;
        let receipt = settled_response(request.message());
        match mode {
            FakeMode::Settle => {
                state.applications += 1;
                Ok(receipt)
            }
            FakeMode::AcceptedThenReceipt => {
                state.applications += 1;
                state.retained_receipt = Some(receipt);
                Ok(accepted_response(request.message()))
            }
            FakeMode::DisconnectAfterWrite => {
                state.applications += 1;
                state.retained_receipt = Some(receipt);
                Err(RuntimeV2TransportFault::DisconnectedAfterWrite)
            }
            FakeMode::DisconnectWithoutReceipt => {
                Err(RuntimeV2TransportFault::DisconnectedAfterWrite)
            }
        }
    }

    fn read_runtime_v2_receipt(
        &mut self,
        request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        let mut state = self.0.borrow_mut();
        state.receipt_reads += 1;
        let mut receipt = state.retained_receipt.clone();
        if let Some(receipt) = receipt.as_mut() {
            receipt.correlation_id = request.message().correlation_id.clone();
        }
        Ok(receipt)
    }
}

fn binding() -> Result<RuntimeV2Binding, String> {
    RuntimeV2Binding::new(
        "instance-1",
        "session-1",
        "lease-1",
        1,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
    )
    .map_err(|error| error.to_string())
}

fn action_request(
    correlation_id: &str,
    operation_id: &str,
    lease_epoch: u64,
    generation: u64,
) -> RuntimeV2Message {
    RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        correlation_id,
        "instance-1",
        "session-1",
        "lease-1",
        lease_epoch,
        generation,
        operation_id,
        RuntimeV2Action::end_turn(),
    )
}

fn reconcile_request(
    correlation_id: &str,
    operation_id: &str,
    generation: u64,
) -> RuntimeV2Message {
    RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        correlation_id,
        "instance-1",
        "session-1",
        "lease-1",
        1,
        generation,
        operation_id,
    )
}

fn settled_response(request: &RuntimeV2Message) -> RuntimeV2Message {
    RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        &request.correlation_id,
        &request.instance_id,
        &request.session_id,
        &request.lease_id,
        request.lease_epoch,
        5,
        request
            .operation_id
            .as_deref()
            .unwrap_or("missing-operation"),
        request
            .action
            .clone()
            .unwrap_or_else(RuntimeV2Action::end_turn),
        RuntimeV2Status::Settled,
        Some(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            3,
            true,
            5,
        )),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
        RuntimeV2MessageKind::ActionResponse,
    )
}

fn accepted_response(request: &RuntimeV2Message) -> RuntimeV2Message {
    RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        &request.correlation_id,
        &request.instance_id,
        &request.session_id,
        &request.lease_id,
        request.lease_epoch,
        request.generation,
        request
            .operation_id
            .as_deref()
            .unwrap_or("missing-operation"),
        request
            .action
            .clone()
            .unwrap_or_else(RuntimeV2Action::end_turn),
        RuntimeV2Status::Accepted,
        Some(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            2,
            true,
            request.generation,
        )),
        None,
        None,
        RuntimeV2MessageKind::ActionResponse,
    )
}

#[test]
fn copied_artifact_is_verified_before_the_fake_lane() -> Result<(), String> {
    verify_runtime_v2_artifact().map_err(|error| error.to_string())
}

#[test]
fn exactly_once_application_and_duplicate_replay() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectAfterWrite);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let request = action_request("corr-1", "op-1", 1, 4);

    let first = ledger
        .submit_action(request.clone())
        .map_err(|error| error.to_string())?;
    let replay = ledger
        .submit_action(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(first, replay);
    assert_eq!(first.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    Ok(())
}

#[test]
fn retry_correlation_replays_without_second_dispatch() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .submit_action(action_request("corr-first", "op-retry-correlation", 1, 4))
        .map_err(|error| error.to_string())?;

    let replay = ledger
        .submit_action(action_request("corr-retry", "op-retry-correlation", 1, 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.status, Some(RuntimeV2Status::Settled));
    assert_eq!(replay.correlation_id, "corr-retry");
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    Ok(())
}

#[test]
fn unknown_reconciles_to_settled_without_dispatch_retry() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectAfterWrite);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let action = action_request("corr-action", "op-timeout", 1, 4);
    let unknown = ledger
        .submit_action(action.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(unknown.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);

    let settled = ledger
        .reconcile(reconcile_request("corr-reconcile", "op-timeout", 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.kind, RuntimeV2MessageKind::ReconcileResponse);
    assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
    assert_eq!(settled.observation.map(|value| value.generation), Some(5));
    assert_eq!(fake.receipt_reads(), 1);

    let replay = ledger
        .submit_action(action)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.status, Some(RuntimeV2Status::Settled));
    assert_eq!(replay.kind, RuntimeV2MessageKind::ActionResponse);
    assert_eq!(replay.correlation_id, "corr-action");
    assert_eq!(fake.dispatches(), 1);
    Ok(())
}

#[test]
fn accepted_reconciles_to_settled_without_dispatch_retry() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::AcceptedThenReceipt);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let action = action_request("corr-accepted", "op-accepted", 1, 4);
    let accepted = ledger
        .submit_action(action.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(accepted.status, Some(RuntimeV2Status::Accepted));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);

    let settled = ledger
        .reconcile(reconcile_request("corr-reconcile", "op-accepted", 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(settled.kind, RuntimeV2MessageKind::ReconcileResponse);
    assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
    assert_eq!(settled.observation.map(|value| value.generation), Some(5));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    assert_eq!(fake.receipt_reads(), 1);
    Ok(())
}

#[test]
fn conflicting_operation_reuse_is_rejected_without_second_application() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .submit_action(action_request("corr-1", "op-1", 1, 4))
        .map_err(|error| error.to_string())?;

    let conflict = ledger
        .submit_action(action_request("corr-2", "op-1", 1, 5))
        .map_err(|error| error.to_string())?;
    assert_eq!(conflict.status, Some(RuntimeV2Status::Rejected));
    assert_eq!(conflict.error_code.as_deref(), Some("idempotency_conflict"));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    Ok(())
}

#[test]
fn stale_epoch_fails_closed_before_dispatch() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let result = ledger.submit_action(action_request("corr-stale", "op-stale", 0, 4));
    assert_eq!(
        result,
        Err(sts2_gateway::RuntimeV2LedgerError::Fence(
            sts2_gateway::RuntimeV2FenceFailure::StaleEpoch,
        ))
    );
    assert_eq!(fake.dispatches(), 0);
    assert_eq!(fake.applications(), 0);
    Ok(())
}

#[test]
fn unknown_without_receipt_is_not_blindly_retried() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectWithoutReceipt);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let action = action_request("corr-action", "op-unknown", 1, 4);
    let first = ledger
        .submit_action(action)
        .map_err(|error| error.to_string())?;
    assert_eq!(first.status, Some(RuntimeV2Status::Unknown));

    let result = ledger
        .reconcile(reconcile_request("corr-reconcile", "op-unknown", 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(result.status, Some(RuntimeV2Status::Unknown));
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.receipt_reads(), 1);
    assert_eq!(fake.applications(), 0);
    Ok(())
}

#[test]
fn cancellation_before_dispatch_is_retained_and_never_forwarded() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let request = action_request("corr-cancel", "op-cancel", 1, 4);
    let cancelled = ledger
        .cancel_before_dispatch(request.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(cancelled.status, Some(RuntimeV2Status::Cancelled));
    let replay = ledger
        .submit_action(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay, cancelled);
    assert_eq!(fake.dispatches(), 0);
    Ok(())
}

#[test]
fn capacity_is_fail_closed() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(1), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .submit_action(action_request("corr-1", "op-1", 1, 4))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        ledger.submit_action(action_request("corr-2", "op-2", 1, 5)),
        Err(sts2_gateway::RuntimeV2LedgerError::CapacityExceeded)
    );
    assert_eq!(fake.dispatches(), 1);
    Ok(())
}

#[path = "runtime_v2/persistence.rs"]
mod persistence;
