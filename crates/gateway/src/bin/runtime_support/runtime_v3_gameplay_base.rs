// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

use super::super::{
    RUNTIME_V3_GAMEPLAY_ARTIFACT, RUNTIME_V3_GAMEPLAY_GENERATOR,
    RUNTIME_V3_GAMEPLAY_MAX_GENERATION, RUNTIME_V3_GAMEPLAY_PROTOCOL_VERSION,
    RUNTIME_V3_GAMEPLAY_SCHEMA_DIGEST, RUNTIME_V3_GAMEPLAY_SCHEMA_SOURCE,
};

const PROVENANCE_FIELDS: &[&str] = &["artifact", "source", "generator"];

pub(super) fn validate_base(
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

pub(super) fn object<'a>(
    value: &'a Value,
    fields: &[&str],
) -> Result<&'a Map<String, Value>, &'static str> {
    let object = value
        .as_object()
        .ok_or("runtime_v3_gameplay_object_invalid")?;
    if !has_exact_fields(object, fields) {
        return Err("runtime_v3_gameplay_unknown_or_missing_field");
    }
    Ok(object)
}

pub(super) fn has_exact_fields(object: &Map<String, Value>, fields: &[&str]) -> bool {
    object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
}

pub(super) fn identity(value: Option<&Value>) -> Result<String, &'static str> {
    let value = value
        .and_then(Value::as_str)
        .ok_or("runtime_v3_gameplay_identity_invalid")?;
    safe_identity(value)
        .then(|| value.to_owned())
        .ok_or("runtime_v3_gameplay_identity_invalid")
}

pub(super) fn optional_identity(value: Option<&Value>) -> Result<Option<String>, &'static str> {
    match value {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => identity(Some(value)).map(Some),
        None => Err("runtime_v3_gameplay_identity_invalid"),
    }
}

pub(super) fn bounded_u64(value: Option<&Value>, maximum: u64) -> Result<u64, &'static str> {
    let value = value
        .and_then(Value::as_u64)
        .ok_or("runtime_v3_gameplay_number_invalid")?;
    (value <= maximum)
        .then_some(value)
        .ok_or("runtime_v3_gameplay_number_out_of_bounds")
}

pub(super) fn is_null(value: Option<&Value>) -> bool {
    value.is_some_and(Value::is_null)
}

pub(super) fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
