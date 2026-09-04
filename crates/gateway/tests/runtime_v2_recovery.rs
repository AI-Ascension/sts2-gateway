// SPDX-License-Identifier: MIT

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2EffectWitness,
    RuntimeV2ForwardRequest, RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation,
    RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault,
};

struct DeferredForwarder {
    initial_status: RuntimeV2Status,
    receipt: Option<RuntimeV2Message>,
    dispatches: usize,
    receipt_reads: usize,
}

fn observation(generation: u64) -> RuntimeV2Observation {
    RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, generation)
}

fn action(operation: &str) -> RuntimeV2Message {
    RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        "correlation-action",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        operation,
        RuntimeV2Action::end_turn(),
    )
}

fn result(request: &RuntimeV2Message, generation: u64) -> RuntimeV2Message {
    let mut response = request.clone();
    response.kind = RuntimeV2MessageKind::ActionResponse;
    response.status = Some(RuntimeV2Status::Settled);
    response.generation = generation;
    response.observation = Some(observation(generation));
    response.effect_witness = Some(RuntimeV2EffectWitness::turn_end_settled(generation));
    response
}

impl RuntimeV2ForwardingPort for DeferredForwarder {
    fn forward_runtime_v2(
        &mut self,
        request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        self.dispatches += 1;
        if request.message().operation_id.as_deref() == Some("operation-newer") {
            return Ok(result(request.message(), 6));
        }
        self.receipt = Some(result(request.message(), 5));
        if self.initial_status == RuntimeV2Status::Unknown {
            return Err(RuntimeV2TransportFault::DisconnectedAfterWrite);
        }
        let mut accepted = result(request.message(), 4);
        accepted.status = Some(RuntimeV2Status::Accepted);
        accepted.effect_witness = None;
        Ok(accepted)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        _request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        self.receipt_reads += 1;
        Ok(self.receipt.clone())
    }
}

fn ledger(status: RuntimeV2Status) -> Result<RuntimeV2Ledger<DeferredForwarder>, String> {
    let binding = RuntimeV2Binding::new("instance-1", "session-1", "lease-1", 1, observation(4))
        .map_err(|error| error.to_string())?;
    RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(4),
        binding,
        DeferredForwarder {
            initial_status: status,
            receipt: None,
            dispatches: 0,
            receipt_reads: 0,
        },
    )
    .map_err(|error| error.to_string())
}

fn reconcile(ledger: &mut RuntimeV2Ledger<DeferredForwarder>) -> Result<RuntimeV2Message, String> {
    let request = RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        "correlation-reconcile",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        ledger.observation().generation,
        "operation-deferred",
    );
    ledger.reconcile(request).map_err(|error| error.to_string())
}

#[test]
fn accepted_action_can_settle_without_another_dispatch() -> Result<(), String> {
    let mut ledger = ledger(RuntimeV2Status::Accepted)?;
    let request = action("operation-deferred");
    let accepted = ledger
        .submit_action(request.clone())
        .map_err(|error| error.to_string())?;
    assert_eq!(accepted.status, Some(RuntimeV2Status::Accepted));
    let settled = reconcile(&mut ledger)?;
    assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
    assert_eq!(settled.generation, 5);
    assert_eq!(settled.correlation_id, "correlation-reconcile");
    let replay = ledger
        .submit_action(request)
        .map_err(|error| error.to_string())?;
    assert_eq!(replay.status, Some(RuntimeV2Status::Settled));
    assert_eq!(replay.correlation_id, "correlation-action");
    assert_eq!(ledger.forwarding_mut().dispatches, 1);
    assert_eq!(ledger.forwarding_mut().receipt_reads, 1);
    Ok(())
}

#[test]
fn older_receipt_settles_operation_without_rewinding_current_observation() -> Result<(), String> {
    for initial in [RuntimeV2Status::Accepted, RuntimeV2Status::Unknown] {
        let mut ledger = ledger(initial)?;
        let request = action("operation-deferred");
        let pending = ledger
            .submit_action(request.clone())
            .map_err(|error| error.to_string())?;
        assert_eq!(pending.status, Some(initial));
        ledger
            .submit_action(action("operation-newer"))
            .map_err(|error| error.to_string())?;
        assert_eq!(ledger.observation().generation, 6);
        let settled = reconcile(&mut ledger)?;
        assert_eq!(settled.status, Some(RuntimeV2Status::Settled));
        assert_eq!(settled.generation, 5);
        assert_eq!(ledger.observation().generation, 6);
        let replay = ledger
            .submit_action(request)
            .map_err(|error| error.to_string())?;
        assert_eq!(replay.generation, 5);
        assert_eq!(ledger.observation().generation, 6);
        assert_eq!(ledger.forwarding_mut().dispatches, 2);
        assert_eq!(ledger.forwarding_mut().receipt_reads, 1);
    }
    Ok(())
}
