// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::rc::Rc;

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2EffectWitness,
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation,
    RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault,
};

#[derive(Clone, Copy)]
enum FakeMode {
    Settle,
    DisconnectAfterWrite,
    Accepted,
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
        let receipt = settled_response(request.message());
        match state.mode {
            FakeMode::Accepted => {
                state.retained_receipt = Some(receipt.clone());
                let mut accepted = receipt;
                accepted.status = Some(RuntimeV2Status::Accepted);
                accepted.generation = request.message().generation;
                accepted.observation = Some(RuntimeV2Observation::new(
                    RuntimeV2CombatPhase::PlayerTurn,
                    2,
                    true,
                    4,
                ));
                accepted.effect_witness = None;
                Ok(accepted)
            }
            FakeMode::Settle => {
                state.applications += 1;
                Ok(receipt)
            }
            FakeMode::DisconnectAfterWrite => {
                state.applications += 1;
                state.retained_receipt = Some(receipt);
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

#[test]
fn identity_and_epoch_are_checked_before_duplicate_replay() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectAfterWrite);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let request = action_request("corr-replay-fence", "op-replay-fence", 1, 4);
    ledger
        .submit_action(request.clone())
        .map_err(|error| error.to_string())?;

    let mut wrong_instance = request.clone();
    wrong_instance.instance_id = String::from("instance-other");
    assert_eq!(
        ledger.submit_action(wrong_instance),
        Err(sts2_gateway::RuntimeV2LedgerError::Fence(
            sts2_gateway::RuntimeV2FenceFailure::WrongInstance,
        ))
    );

    let mut stale_epoch = request;
    stale_epoch.lease_epoch = 0;
    assert_eq!(
        ledger.submit_action(stale_epoch),
        Err(sts2_gateway::RuntimeV2LedgerError::Fence(
            sts2_gateway::RuntimeV2FenceFailure::StaleEpoch,
        ))
    );
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    Ok(())
}

#[test]
fn exact_duplicate_replays_after_generation_advances() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::Settle);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    let request = action_request("corr-stale-replay", "op-stale-replay", 1, 4);
    let original = ledger
        .submit_action(request.clone())
        .map_err(|error| error.to_string())?;
    let replay = ledger
        .submit_action(request.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(replay, original);
    assert_eq!(
        ledger
            .cancel_before_dispatch(request)
            .map_err(|error| error.to_string())?,
        original
    );
    assert_eq!(fake.dispatches(), 1);
    assert_eq!(fake.applications(), 1);
    Ok(())
}

#[test]
fn identity_and_epoch_are_checked_before_receipt_reconciliation() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectAfterWrite);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .submit_action(action_request(
            "corr-receipt-fence",
            "op-receipt-fence",
            1,
            4,
        ))
        .map_err(|error| error.to_string())?;

    let mut wrong_instance = reconcile_request("corr-reconcile", "op-receipt-fence", 4);
    wrong_instance.instance_id = String::from("instance-other");
    assert_eq!(
        ledger.reconcile(wrong_instance),
        Err(sts2_gateway::RuntimeV2LedgerError::Fence(
            sts2_gateway::RuntimeV2FenceFailure::WrongInstance,
        ))
    );
    let mut stale_epoch = reconcile_request("corr-reconcile", "op-receipt-fence", 4);
    stale_epoch.lease_epoch = 0;
    assert_eq!(
        ledger.reconcile(stale_epoch),
        Err(sts2_gateway::RuntimeV2LedgerError::Fence(
            sts2_gateway::RuntimeV2FenceFailure::StaleEpoch,
        ))
    );
    assert_eq!(fake.receipt_reads(), 0);
    Ok(())
}

#[test]
fn stale_generation_is_checked_before_receipt_reconciliation() -> Result<(), String> {
    let fake = FakeForwarder::new(FakeMode::DisconnectAfterWrite);
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
        .map_err(|error| error.to_string())?;
    ledger
        .submit_action(action_request(
            "corr-receipt-generation",
            "op-receipt-generation",
            1,
            4,
        ))
        .map_err(|error| error.to_string())?;
    ledger
        .reconcile(reconcile_request(
            "corr-reconcile",
            "op-receipt-generation",
            4,
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(fake.receipt_reads(), 1);
    assert_eq!(
        ledger.reconcile(reconcile_request(
            "corr-reconcile-again",
            "op-receipt-generation",
            4,
        )),
        Err(sts2_gateway::RuntimeV2LedgerError::StaleGeneration {
            expected: 5,
            actual: 4,
        })
    );
    assert_eq!(fake.receipt_reads(), 1);
    assert_eq!(fake.dispatches(), 1);
    Ok(())
}

#[test]
fn persisted_state_cannot_cross_a_restarted_lease_epoch() -> Result<(), String> {
    let source = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(4),
        binding()?,
        FakeForwarder::new(FakeMode::Settle),
    )
    .map_err(|error| error.to_string())?;
    let persisted = source.persisted_state();
    let replacement_binding = RuntimeV2Binding::new(
        "instance-1",
        "session-1",
        "lease-1",
        2,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
    )
    .map_err(|error| error.to_string())?;
    let mut replacement = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(4),
        replacement_binding,
        FakeForwarder::new(FakeMode::Settle),
    )
    .map_err(|error| error.to_string())?;

    assert_eq!(
        replacement.restore_state(persisted),
        Err(sts2_gateway::RuntimeV2LedgerError::PersistedStateMismatch)
    );
    Ok(())
}

fn refresh(ledger: &mut RuntimeV2Ledger<FakeForwarder>, generation: u64) -> Result<(), String> {
    let request = RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        ledger.observation().generation,
    );
    let response = RuntimeV2Message::state_response(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 4, true, generation),
    );
    ledger
        .accept_state_response(&request, response)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn late_settlement_retains_receipt_without_rewinding_observation() -> Result<(), String> {
    for mode in [FakeMode::Accepted, FakeMode::DisconnectAfterWrite] {
        let fake = FakeForwarder::new(mode);
        let mut ledger =
            RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, fake.clone())
                .map_err(|e| e.to_string())?;
        let request = action_request("corr-action", "op-late", 1, 4);
        ledger
            .submit_action(request.clone())
            .map_err(|e| e.to_string())?;
        refresh(&mut ledger, 6)?;
        let receipt = ledger
            .reconcile(reconcile_request("corr-read", "op-late", 6))
            .map_err(|e| e.to_string())?;
        assert_eq!(receipt.status, Some(RuntimeV2Status::Settled));
        assert_eq!(receipt.generation, 5);
        assert_eq!(ledger.observation().generation, 6);
        let replay = ledger.submit_action(request).map_err(|e| e.to_string())?;
        assert_eq!(replay.status, Some(RuntimeV2Status::Settled));
        assert_eq!(fake.dispatches(), 1);
        assert_eq!(fake.receipt_reads(), 1);
        assert!(refresh(&mut ledger, 5).is_err());
        assert_eq!(ledger.observation().generation, 6);
    }
    Ok(())
}
