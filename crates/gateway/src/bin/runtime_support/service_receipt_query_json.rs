// SPDX-License-Identifier: MIT

use serde_json::Value;

pub(super) const TOP_LEVEL_FIELDS: [&str; 24] = [
    "protocol_version",
    "schema_digest",
    "provenance",
    "correlation_id",
    "instance_id",
    "session_id",
    "lease_id",
    "lease_epoch",
    "kind",
    "operation_id",
    "action_kind",
    "action_fingerprint",
    "run_id",
    "location",
    "actor_id",
    "authority_id",
    "authority_epoch",
    "expected_host_generation",
    "before_host_generation",
    "participant_ids",
    "status",
    "evidence_scope",
    "receipt",
    "error_code",
];
pub(super) const PROVENANCE_FIELDS: [&str; 3] = ["artifact", "source", "generator"];
pub(super) const LOCATION_FIELDS: [&str; 3] = ["act_index", "room_id", "coord"];
pub(super) const COORDINATE_FIELDS: [&str; 2] = ["col", "row"];
pub(super) const RECEIPT_FIELDS: [&str; 7] = [
    "status",
    "after_host_generation",
    "checkpoint_id",
    "state_digest",
    "effect_id",
    "effect_kind",
    "error_code",
];

pub(super) fn canonical_envelope(value: &Value) -> Option<Vec<u8>> {
    let object = value.as_object()?;
    if object.len() != TOP_LEVEL_FIELDS.len() {
        return None;
    }
    let mut output = Vec::new();
    output.push(b'{');
    for (index, field) in TOP_LEVEL_FIELDS.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        append_key(&mut output, field)?;
        match *field {
            "provenance" => append_provenance(&mut output, object.get(*field)?)?,
            "location" => append_location(&mut output, object.get(*field)?)?,
            "participant_ids" => append_participants(&mut output, object.get(*field)?)?,
            "receipt" => append_receipt(&mut output, object.get(*field)?)?,
            _ => append_scalar(&mut output, object.get(*field)?)?,
        }
    }
    output.push(b'}');
    output.push(b'\n');
    Some(output)
}

fn append_key(output: &mut Vec<u8>, key: &str) -> Option<()> {
    output.extend_from_slice(serde_json::to_string(key).ok()?.as_bytes());
    output.push(b':');
    Some(())
}

fn append_scalar(output: &mut Vec<u8>, value: &Value) -> Option<()> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => {
            output.extend_from_slice(serde_json::to_vec(value).ok()?.as_slice());
            Some(())
        }
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                output.extend_from_slice(value.to_string().as_bytes());
                Some(())
            } else if let Some(value) = number.as_i64() {
                output.extend_from_slice(value.to_string().as_bytes());
                Some(())
            } else {
                None
            }
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn append_provenance(output: &mut Vec<u8>, value: &Value) -> Option<()> {
    append_ordered_object(output, value, &PROVENANCE_FIELDS, |field, value, output| {
        append_scalar_member(output, field, value)
    })
}

fn append_location(output: &mut Vec<u8>, value: &Value) -> Option<()> {
    append_ordered_object(
        output,
        value,
        &LOCATION_FIELDS,
        |field, value, output| match field {
            "coord" => {
                if value.is_null() {
                    append_scalar_member(output, field, value)
                } else {
                    append_key(output, field)?;
                    append_ordered_object(
                        output,
                        value,
                        &COORDINATE_FIELDS,
                        |nested, nested_value, nested_output| {
                            append_scalar_member(nested_output, nested, nested_value)
                        },
                    )
                }
            }
            _ => append_scalar_member(output, field, value),
        },
    )
}

fn append_participants(output: &mut Vec<u8>, value: &Value) -> Option<()> {
    let values = value.as_array()?;
    output.push(b'[');
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        append_scalar(output, value)?;
    }
    output.push(b']');
    Some(())
}

fn append_receipt(output: &mut Vec<u8>, value: &Value) -> Option<()> {
    if value.is_null() {
        return append_scalar(output, value);
    }
    append_ordered_object(output, value, &RECEIPT_FIELDS, |field, value, output| {
        append_scalar_member(output, field, value)
    })
}

fn append_ordered_object<F>(
    output: &mut Vec<u8>,
    value: &Value,
    fields: &[&str],
    mut append_value: F,
) -> Option<()>
where
    F: FnMut(&str, &Value, &mut Vec<u8>) -> Option<()>,
{
    let object = value.as_object()?;
    if object.len() != fields.len() {
        return None;
    }
    output.push(b'{');
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        append_value(field, object.get(*field)?, output)?;
    }
    output.push(b'}');
    Some(())
}

fn append_scalar_member(output: &mut Vec<u8>, field: &str, value: &Value) -> Option<()> {
    append_key(output, field)?;
    append_scalar(output, value)
}
