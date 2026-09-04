// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{
    RUNTIME_V3_GAMEPLAY_ACTION_ID, RUNTIME_V3_GAMEPLAY_ARTIFACT, RUNTIME_V3_GAMEPLAY_EFFECT_KIND,
    RUNTIME_V3_GAMEPLAY_GENERATOR, RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX,
    RUNTIME_V3_GAMEPLAY_MAX_ENEMIES, RUNTIME_V3_GAMEPLAY_MAX_ENERGY,
    RUNTIME_V3_GAMEPLAY_MAX_GENERATION, RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT,
    RUNTIME_V3_GAMEPLAY_MAX_TURN_INDEX, RUNTIME_V3_GAMEPLAY_PROTOCOL_VERSION,
    RUNTIME_V3_GAMEPLAY_SCHEMA_DIGEST, RUNTIME_V3_GAMEPLAY_SCHEMA_SOURCE,
};

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
const PROVENANCE_FIELDS: &[&str] = &["artifact", "source", "generator"];
const ACTION_FIELDS: &[&str] = &["action_id", "card_index", "target_id"];
const OBSERVATION_FIELDS: &[&str] = &[
    "combat_phase",
    "turn_index",
    "host_ready",
    "generation",
    "hand_count",
    "energy",
    "draw_pile_count",
    "discard_pile_count",
    "exhaust_pile_count",
    "enemies",
];
const ENEMY_FIELDS: &[&str] = &["target_id", "alive", "hittable"];
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

pub(crate) fn validate_state_response(
    body: &[u8],
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
    correlation_id: &str,
) -> Result<u64, &'static str> {
    let root = super::wire::decode(body).map_err(|_| "runtime_v3_gameplay_response_invalid")?;
    let object = object(&root, TOP_LEVEL_FIELDS)?;
    validate_base(
        object,
        "state_response",
        instance_id,
        session_id,
        lease_id,
        lease_epoch,
        correlation_id,
    )?;
    if !object.get("operation_id").is_some_and(Value::is_null)
        || !object.get("action").is_some_and(Value::is_null)
        || !object.get("status").is_some_and(Value::is_null)
        || !object.get("error_code").is_some_and(Value::is_null)
        || !object.get("effect_witness").is_some_and(Value::is_null)
    {
        return Err("runtime_v3_gameplay_state_shape_invalid");
    }
    let observation = object
        .get("observation")
        .ok_or("runtime_v3_gameplay_state_shape_invalid")?;
    validate_observation(observation)?;
    let generation = bounded_u64(object.get("generation"), RUNTIME_V3_GAMEPLAY_MAX_GENERATION)?;
    if observation
        .as_object()
        .and_then(|value| value.get("generation"))
        .and_then(Value::as_u64)
        != Some(generation)
    {
        return Err("runtime_v3_gameplay_generation_mismatch");
    }
    Ok(generation)
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

fn validate_base(
    object: &Map<String, Value>,
    expected_kind: &str,
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
    correlation_id: &str,
) -> Result<(), &'static str> {
    if object.get("protocol_version").and_then(Value::as_str)
        != Some(RUNTIME_V3_GAMEPLAY_PROTOCOL_VERSION)
        || object.get("schema_digest").and_then(Value::as_str)
            != Some(RUNTIME_V3_GAMEPLAY_SCHEMA_DIGEST)
        || object.get("kind").and_then(Value::as_str) != Some(expected_kind)
        || object.get("instance_id").and_then(Value::as_str) != Some(instance_id)
        || object.get("session_id").and_then(Value::as_str) != Some(session_id)
        || object.get("lease_id").and_then(Value::as_str) != Some(lease_id)
        || object.get("correlation_id").and_then(Value::as_str) != Some(correlation_id)
        || bounded_u64(
            object.get("lease_epoch"),
            RUNTIME_V3_GAMEPLAY_MAX_GENERATION,
        )? != lease_epoch
    {
        return Err("runtime_v3_gameplay_identity_rejected");
    }
    let provenance = object
        .get("provenance")
        .and_then(Value::as_object)
        .ok_or("runtime_v3_gameplay_provenance_invalid")?;
    if !has_exact_fields(provenance, PROVENANCE_FIELDS)
        || provenance.get("artifact").and_then(Value::as_str) != Some(RUNTIME_V3_GAMEPLAY_ARTIFACT)
        || provenance.get("source").and_then(Value::as_str)
            != Some(RUNTIME_V3_GAMEPLAY_SCHEMA_SOURCE)
        || provenance.get("generator").and_then(Value::as_str)
            != Some(RUNTIME_V3_GAMEPLAY_GENERATOR)
    {
        return Err("runtime_v3_gameplay_provenance_invalid");
    }
    for value in [instance_id, session_id, lease_id, correlation_id] {
        if !safe_identity(value) {
            return Err("runtime_v3_gameplay_identity_invalid");
        }
    }
    bounded_u64(object.get("generation"), RUNTIME_V3_GAMEPLAY_MAX_GENERATION).map(|_| ())
}

fn validate_observation(value: &Value) -> Result<(), &'static str> {
    let observation = object(value, OBSERVATION_FIELDS)?;
    let phase = observation
        .get("combat_phase")
        .and_then(Value::as_str)
        .ok_or("runtime_v3_gameplay_observation_invalid")?;
    if !matches!(
        phase,
        "outside_combat" | "combat/player_turn" | "combat/enemy_turn"
    ) || observation
        .get("host_ready")
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err("runtime_v3_gameplay_observation_invalid");
    }
    for (name, maximum) in [
        ("turn_index", u64::from(RUNTIME_V3_GAMEPLAY_MAX_TURN_INDEX)),
        ("hand_count", u64::from(RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX)),
        ("energy", u64::from(RUNTIME_V3_GAMEPLAY_MAX_ENERGY)),
        (
            "draw_pile_count",
            u64::from(RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT),
        ),
        (
            "discard_pile_count",
            u64::from(RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT),
        ),
        (
            "exhaust_pile_count",
            u64::from(RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT),
        ),
        ("generation", RUNTIME_V3_GAMEPLAY_MAX_GENERATION),
    ] {
        bounded_u64(observation.get(name), maximum)?;
    }
    let enemies = observation
        .get("enemies")
        .and_then(Value::as_array)
        .ok_or("runtime_v3_gameplay_observation_invalid")?;
    if enemies.len() > RUNTIME_V3_GAMEPLAY_MAX_ENEMIES {
        return Err("runtime_v3_gameplay_observation_bounds");
    }
    let mut identities = BTreeSet::new();
    for enemy in enemies {
        let enemy = object(enemy, ENEMY_FIELDS)?;
        if !enemy.get("alive").and_then(Value::as_bool).is_some()
            || !enemy.get("hittable").and_then(Value::as_bool).is_some()
        {
            return Err("runtime_v3_gameplay_enemy_invalid");
        }
        let target_id = identity(enemy.get("target_id"))?;
        if !identities.insert(target_id) {
            return Err("runtime_v3_gameplay_duplicate_target");
        }
    }
    Ok(())
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

fn object<'a>(value: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, &'static str> {
    let object = value
        .as_object()
        .ok_or("runtime_v3_gameplay_object_invalid")?;
    if !has_exact_fields(object, fields) {
        return Err("runtime_v3_gameplay_unknown_or_missing_field");
    }
    Ok(object)
}

fn has_exact_fields(object: &Map<String, Value>, fields: &[&str]) -> bool {
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}

fn identity(value: Option<&Value>) -> Result<String, &'static str> {
    let value = value
        .and_then(Value::as_str)
        .ok_or("runtime_v3_gameplay_identity_invalid")?;
    safe_identity(value)
        .then(|| value.to_owned())
        .ok_or("runtime_v3_gameplay_identity_invalid")
}

fn optional_identity(value: Option<&Value>) -> Result<Option<String>, &'static str> {
    match value {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => identity(Some(value)).map(Some),
        None => Err("runtime_v3_gameplay_identity_invalid"),
    }
}

fn bounded_u64(value: Option<&Value>, maximum: u64) -> Result<u64, &'static str> {
    let value = value
        .and_then(Value::as_u64)
        .ok_or("runtime_v3_gameplay_number_invalid")?;
    (value <= maximum)
        .then_some(value)
        .ok_or("runtime_v3_gameplay_number_out_of_bounds")
}

fn is_null(value: Option<&Value>) -> bool {
    value.is_some_and(Value::is_null)
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
