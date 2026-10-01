// SPDX-License-Identifier: MIT

//! Direct coverage for the individual clauses of the Runtime-v2 message-shape predicates.
//!
//! The frozen contract fixes which members each message kind may carry. `validate_state_request`,
//! `validate_state_response`, `validate_action_request`, `validate_reconcile_request` and
//! `validate_result` each spell that out as a conjunction of clauses, and
//! `result_fields_match_status` fixes which members each of the five result statuses requires.
//!
//! Every one of those clauses could be deleted individually without a single test going red, so a
//! message could claim members its kind does not permit - or omit ones its status requires - and
//! still be admitted. Each test below therefore drives exactly one clause out of compliance and
//! requires the refusal, so no test can be satisfied by a neighbouring clause.
//!
//! `ResultShape` is also raised by the ledger when a response does not match its request, which is
//! a distinct check with its own coverage; these tests target the wire-shape predicates.
//!
//! One clause is deliberately *not* claimed here. A result message with no `status` at all is
//! already refused under the unmutated predicate, and it stays refused when the explicit
//! `status.is_some()` clause is deleted, because `result_fields_match_status` returns false for a
//! missing status through its own `None => false` arm. Deleting that clause therefore changes
//! nothing observable, so a test asserting on it could only ever pass by accident. That redundancy
//! is recorded in `a_settled_result_keeps_status_and_witness` below so it is visible rather than
//! silently assumed.

use sts2_gateway::{
    RuntimeV2Action, RuntimeV2CombatPhase, RuntimeV2EffectWitness, RuntimeV2Message,
    RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation, RuntimeV2Status,
    RuntimeV2ValidationError,
};

fn observation(generation: u64) -> RuntimeV2Observation {
    RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, generation)
}

fn state_request() -> RuntimeV2Message {
    RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        "corr-shape",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
    )
}

fn state_response() -> RuntimeV2Message {
    RuntimeV2Message::state_response(
        RuntimeV2Metadata::new(),
        "corr-shape",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        observation(4),
    )
}

fn action_request() -> RuntimeV2Message {
    RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        "corr-shape",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-shape",
        RuntimeV2Action::end_turn(),
    )
}

fn reconcile_request() -> RuntimeV2Message {
    RuntimeV2Message::reconcile_request(
        RuntimeV2Metadata::new(),
        "corr-shape",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-shape",
    )
}

/// Builds an action result whose nullable members are set exactly as given.
#[allow(clippy::too_many_arguments)]
fn result(
    status: RuntimeV2Status,
    generation: u64,
    operation_id: Option<&str>,
    action: Option<RuntimeV2Action>,
    observation: Option<RuntimeV2Observation>,
    error_code: Option<&str>,
    effect_witness: Option<RuntimeV2EffectWitness>,
) -> RuntimeV2Message {
    let mut message = RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        "corr-shape",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        generation,
        "op-shape",
        RuntimeV2Action::end_turn(),
        status,
        observation,
        error_code.map(str::to_owned),
        effect_witness,
        RuntimeV2MessageKind::ActionResponse,
    );
    message.operation_id = operation_id.map(str::to_owned);
    message.action = action;
    message
}

/// A result that satisfies every clause for its status, used as the base for each negative.
fn settled() -> RuntimeV2Message {
    result(
        RuntimeV2Status::Settled,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
    )
}

/// Requires the message to be refused specifically for its shape.
fn refused_shape(message: &RuntimeV2Message) {
    assert_eq!(
        message.validate(),
        Err(RuntimeV2ValidationError::ResultShape),
        "the message must be refused for its shape"
    );
}

/// Requires the message to be accepted, so the negatives below cannot pass vacuously.
fn accepted(message: &RuntimeV2Message) {
    assert_eq!(message.validate(), Ok(()), "the message must be accepted");
}

#[test]
fn a_state_request_carries_no_result_members() {
    accepted(&state_request());

    // operation_id must be absent from a state request.
    let mut with_operation = state_request();
    with_operation.operation_id = Some(String::from("op-shape"));
    refused_shape(&with_operation);

    // ...and so must observation.
    let mut with_observation = state_request();
    with_observation.observation = Some(observation(4));
    refused_shape(&with_observation);
}

#[test]
fn a_state_response_carries_only_its_matching_observation() {
    accepted(&state_response());

    // The observation generation must agree with the message generation.
    let mut mismatched = state_response();
    mismatched.generation = 6;
    refused_shape(&mismatched);
}

#[test]
fn an_action_request_carries_an_operation() {
    accepted(&action_request());

    // operation_id must be present on an action request.
    let mut without_operation = action_request();
    without_operation.operation_id = None;
    refused_shape(&without_operation);
}

#[test]
fn a_reconcile_request_carries_an_operation_and_never_an_action() {
    accepted(&reconcile_request());

    // operation_id must be present on a reconcile request.
    let mut without_operation = reconcile_request();
    without_operation.operation_id = None;
    refused_shape(&without_operation);

    // ...and action must be absent, because a reconcile carries no action of its own.
    let mut with_action = reconcile_request();
    with_action.action = Some(RuntimeV2Action::end_turn());
    refused_shape(&with_action);
}

#[test]
fn a_result_carries_an_operation_an_action_and_a_status() {
    accepted(&settled());

    let without_operation = result(
        RuntimeV2Status::Settled,
        5,
        None,
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
    );
    refused_shape(&without_operation);

    let without_action = result(
        RuntimeV2Status::Settled,
        5,
        Some("op-shape"),
        None,
        Some(observation(5)),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
    );
    refused_shape(&without_action);

    let mut without_status = settled();
    without_status.status = None;
    refused_shape(&without_status);
}

#[test]
fn an_accepted_result_carries_no_error_code() {
    let accepted_result = result(
        RuntimeV2Status::Accepted,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        None,
        None,
    );
    accepted(&accepted_result);

    // An accepted result must not also carry an error code.
    let with_error = result(
        RuntimeV2Status::Accepted,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        Some("forged_error"),
        None,
    );
    refused_shape(&with_error);
}

#[test]
fn a_settled_result_requires_a_witness_at_the_same_generation() {
    accepted(&settled());

    // The witness generation must agree with the message generation; a witness that merely exists
    // is not enough, or a stale witness could attest to an effect that never happened.
    let stale_witness = result(
        RuntimeV2Status::Settled,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        None,
        Some(RuntimeV2EffectWitness::turn_end_settled(4)),
    );
    refused_shape(&stale_witness);
}

#[test]
fn a_settled_result_keeps_status_and_witness() {
    // Documents the redundancy noted in the module comment: a settled result missing its status is
    // refused, and so is one missing its witness. Both routes are asserted together so the
    // refusal property is pinned even though no single clause owns it exclusively.
    accepted(&settled());

    let mut without_status = settled();
    without_status.status = None;
    refused_shape(&without_status);

    let without_witness = result(
        RuntimeV2Status::Settled,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        None,
        None,
    );
    refused_shape(&without_witness);
}

#[test]
fn a_rejected_result_carries_no_effect_witness() {
    let rejected = result(
        RuntimeV2Status::Rejected,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        Some("rejected"),
        None,
    );
    accepted(&rejected);

    let rejected_with_witness = result(
        RuntimeV2Status::Rejected,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        Some("rejected"),
        Some(RuntimeV2EffectWitness::turn_end_settled(5)),
    );
    refused_shape(&rejected_with_witness);
}

#[test]
fn an_unknown_result_carries_no_observation() {
    let unknown = result(
        RuntimeV2Status::Unknown,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        None,
        Some("unknown"),
        None,
    );
    accepted(&unknown);

    // An unknown result must not also carry an observation, because it asserts nothing was observed.
    let unknown_with_observation = result(
        RuntimeV2Status::Unknown,
        5,
        Some("op-shape"),
        Some(RuntimeV2Action::end_turn()),
        Some(observation(5)),
        Some("unknown"),
        None,
    );
    refused_shape(&unknown_with_observation);
}
