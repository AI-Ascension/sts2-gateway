// SPDX-License-Identifier: MIT

use super::super::configuration::valid_digest;
use super::super::{GenerationRecord, NativeProcessError};

fn validate(record: &GenerationRecord) -> Result<(), NativeProcessError> {
    let unit_id = record
        .unit
        .strip_prefix("sts2-gateway-")
        .and_then(|value| value.strip_suffix(".service"));
    if !valid_digest(&record.key)
        || !valid_digest(&record.binding_digest)
        || record.handle == 0
        || record.handle > i64::MAX as u64
        || uuid::Uuid::parse_str(&record.boot_id).is_err()
        || unit_id.is_none_or(|value| value.len() != 32 || uuid::Uuid::parse_str(value).is_err())
        || record
            .invocation
            .as_ref()
            .is_some_and(|value| value.len() != 32 || uuid::Uuid::parse_str(value).is_err())
        || record
            .control_group
            .as_ref()
            .is_some_and(|value| !super::super::identity::valid_group(value, &record.unit))
        || (!record.dispatched && (record.stopping || record.absent || record.invocation.is_some()))
        || (record.invocation.is_some() != record.control_group.is_some())
        || (record.stopping && record.identity.is_none())
        || (record.absent && record.control_group.is_none())
        || (record.identity.is_some() && record.invocation.is_none())
        || record.identity.as_ref().is_some_and(|identity| {
            identity.process().value() != record.handle
                || !identity.matches_profile(identity.instance_id(), record.profile)
        })
    {
        return Err(NativeProcessError::Registry);
    }
    Ok(())
}

pub(super) fn encode(record: &GenerationRecord) -> Result<Vec<u8>, NativeProcessError> {
    validate(record)?;
    let bytes = serde_json::to_vec(record).map_err(|_| NativeProcessError::Registry)?;
    if bytes.len() > 16_384 {
        return Err(NativeProcessError::Registry);
    }
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<GenerationRecord, NativeProcessError> {
    if bytes.len() > 16_384 {
        return Err(NativeProcessError::Registry);
    }
    let record = serde_json::from_slice(bytes).map_err(|_| NativeProcessError::Registry)?;
    validate(&record)?;
    Ok(record)
}

pub(super) fn decode_row(
    row: (i64, String, Vec<u8>),
) -> Result<GenerationRecord, NativeProcessError> {
    let record = decode(&row.2)?;
    if row.0 <= 0 || record.handle != row.0 as u64 || record.key != row.1 {
        return Err(NativeProcessError::Registry);
    }
    Ok(record)
}
