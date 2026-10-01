// SPDX-License-Identifier: MIT

//! Direct coverage for the individual clauses of the Runtime-v2 validation predicates.
//!
//! `RuntimeV2Message::validate` and `RuntimeV2Binding::new` together enforce the frozen Runtime-v2
//! contract: a message may claim only the fixed protocol version, schema digest, provenance, action
//! identity and effect-witness kind, its observation must sit inside the bounded turn/generation
//! range, and every identity string must be a safe bounded token. No test in this repository
//! asserted any of those refusals by name, so each one could be deleted with the whole suite still
//! green.
//!
//! Every negative asserts the specific refusal variant *and* that the ledger dispatched nothing,
//! because the property that matters operationally is that a malformed message is refused before it
//! reaches the host. The first test is the positive control: a validator that refused everything
//! would satisfy every negative below and prove nothing.

use std::cell::Cell;
use std::rc::Rc;

use sts2_gateway::{
    RUNTIME_V2_MAX_GENERATION, RUNTIME_V2_MAX_TURN_INDEX, RuntimeV2Action, RuntimeV2Authority,
    RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2EffectWitness, RuntimeV2ForwardRequest,
    RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError,
    RuntimeV2Message, RuntimeV2Metadata, RuntimeV2Observation, RuntimeV2Provenance,
    RuntimeV2ReceiptRequest, RuntimeV2Status, RuntimeV2TransportFault, RuntimeV2ValidationError,
};

/// Records whether the ledger ever reached the forwarding port.
#[derive(Clone)]
struct CountingPort {
    dispatches: Rc<Cell<usize>>,
}

impl RuntimeV2ForwardingPort for CountingPort {
    fn forward_runtime_v2(
        &mut self,
        _request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault> {
        self.dispatches.set(self.dispatches.get() + 1);
        Err(RuntimeV2TransportFault::DisconnectedBeforeWrite)
    }

    fn read_runtime_v2_receipt(
        &mut self,
        _request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault> {
        Ok(None)
    }
}

fn observation() -> RuntimeV2Observation {
    RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4)
}

fn binding() -> Result<RuntimeV2Binding, String> {
    RuntimeV2Binding::new("instance-1", "session-1", "lease-1", 1, observation())
        .map_err(|error| error.to_string())
}

fn action_request(metadata: RuntimeV2Metadata, action: RuntimeV2Action) -> RuntimeV2Message {
    RuntimeV2Message::action_request(
        metadata,
        "corr-validation",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-validation",
        action,
    )
}

/// Submits one request and requires the ledger to refuse it with a validation error.
fn refusal(request: RuntimeV2Message, expected: RuntimeV2ValidationError) -> Result<(), String> {
    let dispatches = Rc::new(Cell::new(0));
    let port = CountingPort {
        dispatches: dispatches.clone(),
    };
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, port)
        .map_err(|error| error.to_string())?;
    assert_eq!(
        ledger.submit_action(request),
        Err(RuntimeV2LedgerError::InvalidRequest(expected)),
        "the ledger must refuse the request with the specific validation variant"
    );
    assert_eq!(
        dispatches.get(),
        0,
        "a refused request must never reach the forwarding port"
    );
    Ok(())
}

#[test]
fn a_well_formed_action_request_is_admitted() -> Result<(), String> {
    // The positive control. Without it a validator that refused everything would satisfy every
    // negative below and this file would prove nothing.
    let dispatches = Rc::new(Cell::new(0));
    let port = CountingPort {
        dispatches: dispatches.clone(),
    };
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, port)
        .map_err(|error| error.to_string())?;
    let raw = ledger.submit_action(action_request(
        RuntimeV2Metadata::new(),
        RuntimeV2Action::end_turn(),
    ));
    assert!(
        !matches!(raw, Err(RuntimeV2LedgerError::InvalidRequest(_))),
        "a well-formed request must not be refused as invalid"
    );
    assert_eq!(dispatches.get(), 1, "a valid request must be dispatched");
    Ok(())
}

#[test]
fn a_forged_protocol_version_is_refused() -> Result<(), String> {
    let mut metadata = RuntimeV2Metadata::new();
    metadata.protocol_version = String::from("runtime-v1");
    refusal(
        action_request(metadata, RuntimeV2Action::end_turn()),
        RuntimeV2ValidationError::Metadata,
    )
}

#[test]
fn a_forged_schema_digest_is_refused() -> Result<(), String> {
    let mut metadata = RuntimeV2Metadata::new();
    metadata.schema_digest = "0".repeat(64);
    refusal(
        action_request(metadata, RuntimeV2Action::end_turn()),
        RuntimeV2ValidationError::Metadata,
    )
}

#[test]
fn a_forged_provenance_is_refused() -> Result<(), String> {
    let mut metadata = RuntimeV2Metadata::new();
    metadata.provenance = RuntimeV2Provenance {
        artifact: String::from("sts2-protocol/forged"),
        source: String::from("schemas/forged.json"),
        generator: String::from("machine"),
    };
    refusal(
        action_request(metadata, RuntimeV2Action::end_turn()),
        RuntimeV2ValidationError::Provenance,
    )
}

#[test]
fn a_forged_action_identity_is_refused() -> Result<(), String> {
    refusal(
        action_request(
            RuntimeV2Metadata::new(),
            RuntimeV2Action::new("skip_reward"),
        ),
        RuntimeV2ValidationError::ActionBounds,
    )
}

#[test]
fn an_out_of_range_turn_index_is_refused() -> Result<(), String> {
    let mut request = action_request(RuntimeV2Metadata::new(), RuntimeV2Action::end_turn());
    request.observation = Some(RuntimeV2Observation::new(
        RuntimeV2CombatPhase::PlayerTurn,
        RUNTIME_V2_MAX_TURN_INDEX + 1,
        true,
        4,
    ));
    refusal(request, RuntimeV2ValidationError::ObservationBounds)
}

#[test]
fn an_overflowing_generation_is_refused() -> Result<(), String> {
    let mut request = action_request(RuntimeV2Metadata::new(), RuntimeV2Action::end_turn());
    request.generation = u64::MAX;
    refusal(request, RuntimeV2ValidationError::GenerationBounds)
}

#[test]
fn an_unsafe_identity_is_refused() -> Result<(), String> {
    let mut request = action_request(RuntimeV2Metadata::new(), RuntimeV2Action::end_turn());
    request.session_id = String::from("session with spaces");
    refusal(request, RuntimeV2ValidationError::InvalidIdentity)
}

#[test]
fn an_overflowing_observation_generation_is_refused() -> Result<(), String> {
    // `RuntimeV2Observation::validate` bounds the generation independently of the message's own
    // generation field, so this guard is reachable on its own and must be asserted directly rather
    // than relying on the message-level check above to cover it.
    let over = RuntimeV2Observation::new(
        RuntimeV2CombatPhase::PlayerTurn,
        2,
        true,
        RUNTIME_V2_MAX_GENERATION + 1,
    );
    assert_eq!(
        over.validate(),
        Err(RuntimeV2ValidationError::GenerationBounds),
        "an observation past the frozen generation bound must be refused"
    );
    Ok(())
}

#[test]
fn an_overflowing_binding_lease_epoch_is_refused() -> Result<(), String> {
    // The binding's lease epoch carries its own bound; nothing else validates it.
    assert_eq!(
        RuntimeV2Binding::new(
            "instance-1",
            "session-1",
            "lease-1",
            RUNTIME_V2_MAX_GENERATION + 1,
            observation(),
        ),
        Err(RuntimeV2ValidationError::GenerationBounds),
        "a lease epoch past the frozen generation bound must be refused"
    );
    Ok(())
}

#[test]
fn an_overflowing_authority_lease_epoch_is_refused() -> Result<(), String> {
    // The recovery authority key bounds its lease epoch separately from both the binding and the
    // message, so deleting that guard is invisible to every other suite.
    assert_eq!(
        RuntimeV2Authority::new(
            "instance-1",
            "session-1",
            "lease-1",
            RUNTIME_V2_MAX_GENERATION + 1,
            "boot-1",
        ),
        Err(RuntimeV2ValidationError::GenerationBounds),
        "an authority lease epoch past the frozen generation bound must be refused"
    );
    Ok(())
}

#[test]
fn a_forged_effect_witness_is_refused() -> Result<(), String> {
    let mut request = action_request(RuntimeV2Metadata::new(), RuntimeV2Action::end_turn());
    request.status = Some(RuntimeV2Status::Settled);
    request.observation = Some(RuntimeV2Observation::new(
        RuntimeV2CombatPhase::PlayerTurn,
        3,
        true,
        5,
    ));
    request.effect_witness = Some(RuntimeV2EffectWitness {
        kind: String::from("forged_witness"),
        generation: 5,
    });
    refusal(request, RuntimeV2ValidationError::EffectBounds)
}
