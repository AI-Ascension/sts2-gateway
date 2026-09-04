// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::{
    RUNTIME_V3_GAMEPLAY_ACTION_ID, RUNTIME_V3_GAMEPLAY_EFFECT_KIND,
    RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX, RUNTIME_V3_GAMEPLAY_MAX_GENERATION,
};

#[path = "runtime_v3_gameplay_base.rs"]
mod base;
#[path = "runtime_v3_gameplay_observation.rs"]
mod observation;
use base::{
    bounded_u64, has_exact_fields, identity, is_null, object, optional_identity, validate_base,
};
use observation::validate_observation;
pub(crate) use observation::validate_state_response;

const TOP_LEVEL_FIELDS: &[&str] = &[
    "protocol_version",
    "schema_digest",
    "provenance",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "generation",
    "kind",
    "operation_id",
    "observation",
    "action",
    "status",
    "error_code",
    "effect_witness",
];
const ACTION_FIELDS: &[&str] = &["action_id", "card_index", "target_id"];
const WITNESS_FIELDS: &[&str] = &["kind", "generation", "card_index", "target_id"];

#[derive(Clone, Debug)]
pub(crate) struct ParsedAction {
    pub(crate) canonical_body: Vec<u8>,
    pub(crate) operation_id: String,
    pub(crate) generation: u64,
    pub(crate) card_index: u16,
    pub(crate) target_id: Option<String>,
}

pub(crate) fn parse_action_request(
    body: &[u8],
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
    correlation_id: &str,
) -> Result<ParsedAction, &'static str> {
    let root = super::wire::decode(body).map_err(|_| "runtime_v3_gameplay_request_invalid")?;
    let object = object(&root, TOP_LEVEL_FIELDS)?;
    validate_base(
        object,
        "action_request",
        instance_id,
        session_id,
        lease_id,
        lease_epoch,
        correlation_id,
    )?;
    if !is_null(object.get("observation"))
        || !is_null(object.get("status"))
        || !is_null(object.get("error_code"))
        || !is_null(object.get("effect_witness"))
    {
        return Err("runtime_v3_gameplay_request_shape_invalid");
    }
    let operation_id = identity(object.get("operation_id"))?;
    let action = object
        .get("action")
        .and_then(Value::as_object)
        .ok_or("runtime_v3_gameplay_action_invalid")?;
    if !has_exact_fields(action, ACTION_FIELDS)
        || action.get("action_id").and_then(Value::as_str) != Some(RUNTIME_V3_GAMEPLAY_ACTION_ID)
    {
        return Err("runtime_v3_gameplay_action_invalid");
    }
    let card_index = bounded_u64(
        action.get("card_index"),
        u64::from(RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX),
    )? as u16;
    let target_id = optional_identity(action.get("target_id"))?;
    Ok(ParsedAction {
        canonical_body: super::wire::canonical(&root)?,
        operation_id,
        generation: bounded_u64(object.get("generation"), RUNTIME_V3_GAMEPLAY_MAX_GENERATION)?,
        card_index,
        target_id,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_result_response(
    body: &[u8],
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
    correlation_id: &str,
    expected_operation_id: &str,
    expected_action: &ParsedAction,
) -> Result<u64, &'static str> {
    let root = super::wire::decode(body).map_err(|_| "runtime_v3_gameplay_response_invalid")?;
    let object = object(&root, TOP_LEVEL_FIELDS)?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("runtime_v3_gameplay_response_invalid")?;
    if kind != "action_response" && kind != "reconcile_response" {
        return Err("runtime_v3_gameplay_response_kind_invalid");
    }
    validate_base(
        object,
        kind,
        instance_id,
        session_id,
        lease_id,
        lease_epoch,
        correlation_id,
    )?;
    if identity(object.get("operation_id"))? != expected_operation_id {
        return Err("runtime_v3_gameplay_operation_mismatch");
    }
    let action = object
        .get("action")
        .and_then(Value::as_object)
        .ok_or("runtime_v3_gameplay_action_invalid")?;
    if !has_exact_fields(action, ACTION_FIELDS)
        || action.get("action_id").and_then(Value::as_str) != Some(RUNTIME_V3_GAMEPLAY_ACTION_ID)
        || bounded_u64(
            action.get("card_index"),
            u64::from(RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX),
        )? != u64::from(expected_action.card_index)
        || optional_identity(action.get("target_id"))? != expected_action.target_id
    {
        return Err("runtime_v3_gameplay_action_mismatch");
    }
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .ok_or("runtime_v3_gameplay_status_invalid")?;
    let generation = bounded_u64(object.get("generation"), RUNTIME_V3_GAMEPLAY_MAX_GENERATION)?;
    let observation = object.get("observation");
    let error_code = object.get("error_code");
    let witness = object.get("effect_witness");
    match status {
        "accepted" => {
            validate_observation(observation.ok_or("runtime_v3_gameplay_result_invalid")?)?;
            if !is_null(error_code) || !is_null(witness) {
                return Err("runtime_v3_gameplay_result_invalid");
            }
        }
        "settled" => {
            validate_observation(observation.ok_or("runtime_v3_gameplay_result_invalid")?)?;
            if !is_null(error_code) || generation <= expected_action.generation {
                return Err("runtime_v3_gameplay_result_invalid");
            }
            validate_witness(witness, expected_action, generation)?;
        }
        "rejected" | "cancelled" => {
            validate_observation(observation.ok_or("runtime_v3_gameplay_result_invalid")?)?;
            identity(error_code)?;
            if !is_null(witness) {
                return Err("runtime_v3_gameplay_result_invalid");
            }
        }
        "unknown" => {
            if !is_null(observation) || identity(error_code).is_err() || !is_null(witness) {
                return Err("runtime_v3_gameplay_result_invalid");
            }
        }
        _ => return Err("runtime_v3_gameplay_status_invalid"),
    }
    if observation
        .and_then(Value::as_object)
        .and_then(|value| value.get("generation"))
        .and_then(Value::as_u64)
        .is_some_and(|value| value != generation)
    {
        return Err("runtime_v3_gameplay_generation_mismatch");
    }
    Ok(generation)
}

fn validate_witness(
    value: Option<&Value>,
    expected_action: &ParsedAction,
    expected_generation: u64,
) -> Result<(), &'static str> {
    let witness = object(
        value.ok_or("runtime_v3_gameplay_witness_invalid")?,
        WITNESS_FIELDS,
    )?;
    if witness.get("kind").and_then(Value::as_str) != Some(RUNTIME_V3_GAMEPLAY_EFFECT_KIND)
        || bounded_u64(
            witness.get("card_index"),
            u64::from(RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX),
        )? != u64::from(expected_action.card_index)
        || optional_identity(witness.get("target_id"))? != expected_action.target_id
        || bounded_u64(
            witness.get("generation"),
            RUNTIME_V3_GAMEPLAY_MAX_GENERATION,
        )? != expected_generation
    {
        return Err("runtime_v3_gameplay_witness_mismatch");
    }
    Ok(())
}
