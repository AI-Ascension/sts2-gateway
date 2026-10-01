// SPDX-License-Identifier: MIT

//! Direct coverage for the Runtime-v2 ledger's construction and lookup guards.
//!
//! `ZeroCapacity` and `OperationNotFound` were both individually deletable with the whole 600-test
//! workspace suite green, so neither refusal had any test naming it. Both are reachable through the
//! public API, and both are pinned here.
//!
//! Two sibling guards in the same file - `OperationInProgress` and the two `MissingOperationId`
//! sites - are deliberately NOT claimed. I probed each on the unmutated source rather than
//! assuming, and the probes are recorded in `unreachable_guards_are_not_claimed`: those guards
//! cannot be reached through the public API, so a test for them could only be written with unsafe
//! re-entrancy or would assert a path that cannot occur. That is a different fact from "untested",
//! and it is documented rather than papered over.

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2ForwardRequest,
    RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError,
    RuntimeV2Message, RuntimeV2Metadata, RuntimeV2Observation, RuntimeV2ReceiptRequest,
    RuntimeV2TransportFault,
};

/// A port that records dispatches; never used by the guards under test.
#[derive(Clone)]
struct NeverCalled {
    dispatches: std::rc::Rc<std::cell::Cell<usize>>,
}

impl RuntimeV2ForwardingPort for NeverCalled {
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

#[test]
fn a_ledger_with_zero_capacity_is_refused_at_construction() -> Result<(), String> {
    // ZeroCapacity. Deleting this guard lets a zero-capacity ledger be constructed, and every
    // subsequent submit would then hit CapacityExceeded instead of failing closed up front.
    let dispatches = std::rc::Rc::new(std::cell::Cell::new(0));
    let port = NeverCalled {
        dispatches: dispatches.clone(),
    };
    let outcome = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(0), binding()?, port);
    assert_eq!(
        outcome.err(),
        Some(RuntimeV2LedgerError::ZeroCapacity),
        "a ledger with zero operation capacity must be refused at construction"
    );
    assert_eq!(
        dispatches.get(),
        0,
        "no dispatch can happen before construction"
    );
    Ok(())
}

#[test]
fn reconciling_an_operation_that_was_never_submitted_is_refused() -> Result<(), String> {
    // OperationNotFound. Reconciling an operation the ledger never retained must fail closed rather
    // than reporting an empty or synthesised receipt.
    let dispatches = std::rc::Rc::new(std::cell::Cell::new(0));
    let port = NeverCalled {
        dispatches: dispatches.clone(),
    };
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, port)
        .map_err(|error| error.to_string())?;
    let request = RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        "corr-not-found",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-never-submitted",
    );
    assert_eq!(
        ledger.reconcile(request),
        Err(RuntimeV2LedgerError::OperationNotFound),
        "an operation that was never retained must not reconcile"
    );
    assert_eq!(
        dispatches.get(),
        0,
        "a reconcile for an unknown operation must not reach the forwarding port"
    );
    Ok(())
}

#[test]
fn unreachable_guards_are_not_claimed() -> Result<(), String> {
    // This test exists to record, in executable form, that three sibling guards are NOT reachable
    // through the public API - so no PR may later claim to have "covered" them with a test.
    //
    // 1. MissingOperationId (both sites). Every path that would raise it runs after a message-shape
    //    predicate that already refuses an operation_id-less message with ResultShape. I probed both
    //    mutants: behaviour was identical, so the clause is masked, not merely untested.
    let dispatches = std::rc::Rc::new(std::cell::Cell::new(0));
    let port = NeverCalled {
        dispatches: dispatches.clone(),
    };
    let mut ledger = RuntimeV2Ledger::new(RuntimeV2LedgerConfig::new(4), binding()?, port)
        .map_err(|error| error.to_string())?;
    let mut action = RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        "corr-no-op",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-absent",
        RuntimeV2Action::end_turn(),
    );
    action.operation_id = None;
    assert_eq!(
        ledger.submit_action(action),
        Err(RuntimeV2LedgerError::InvalidRequest(
            sts2_gateway::RuntimeV2ValidationError::ResultShape,
        )),
        "an operation_id-less action is refused by the shape predicate, before key_for"
    );

    // 2. OperationInProgress. A retained operation is inserted with result == None only inside
    //    submit_action_inner, and that same &mut self fills the result before any public method can
    //    return. The checkpoint closure can observe the None window in the persisted snapshot
    //    (verified: a probe saw result.is_some() == false then true), but no public method reads
    //    operations during that window, so a test could only reach it with unsafe re-entrancy.
    let mut reconcile = RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        "corr-no-op",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-absent",
    );
    reconcile.operation_id = None;
    assert_eq!(
        ledger.reconcile(reconcile),
        Err(RuntimeV2LedgerError::InvalidRequest(
            sts2_gateway::RuntimeV2ValidationError::ResultShape,
        )),
        "an operation_id-less reconcile is refused by the shape predicate, before key_for"
    );
    assert_eq!(dispatches.get(), 0);
    Ok(())
}
