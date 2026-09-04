// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeSet;

use super::super::{
    RUNTIME_V3_GAMEPLAY_MAX_CARD_INDEX, RUNTIME_V3_GAMEPLAY_MAX_ENEMIES,
    RUNTIME_V3_GAMEPLAY_MAX_ENERGY, RUNTIME_V3_GAMEPLAY_MAX_GENERATION,
    RUNTIME_V3_GAMEPLAY_MAX_PILE_COUNT, RUNTIME_V3_GAMEPLAY_MAX_TURN_INDEX,
};
use super::TOP_LEVEL_FIELDS;
use super::base::{bounded_u64, identity, object, validate_base};

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

pub(crate) fn validate_state_response(
    body: &[u8],
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
    correlation_id: &str,
) -> Result<u64, &'static str> {
    let root =
        super::super::wire::decode(body).map_err(|_| "runtime_v3_gameplay_response_invalid")?;
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

pub(super) fn validate_observation(value: &Value) -> Result<(), &'static str> {
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
