// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use super::provisioning_types::{
    UserDataProvisioningError as Error, UserDataProvisioningRecord as Record,
    UserDataProvisioningStatus as Status,
};
use super::provisioning_validation::{guidance, unknown_guidance, validate_request};

pub(super) const MAX_ALLOCATIONS: usize = 64;
pub(super) const MAX_RECORD_BYTES: usize = 8192;

pub(super) fn validate(record: &Record) -> Result<(), Error> {
    validate_request(
        &record.context,
        &record.operation_id,
        &record.launch_profile.profile_id,
    )?;
    if record.descriptor.validate().is_err()
        || record.launch_profile.validate().is_err()
        || record.descriptor.identity != record.launch_profile.user_data
        || record.descriptor.identity.value() > MAX_ALLOCATIONS as u64
        || record.descriptor.provenance.instance_id != record.context.instance_id
        || record.descriptor.provenance.operation_id != record.operation_id
        || super::user_data_provenance::validate(&record.descriptor.provenance).is_err()
    {
        return Err(Error::InvalidRequest);
    }
    let guidance_valid = match record.status {
        Status::Pending | Status::Created | Status::Rejected => record.guidance.is_none(),
        Status::Unknown => record.guidance == Some(unknown_guidance()),
        Status::Blocked => [
            "unknown existing contents",
            "allocation path traversal rejected",
            "allocation symlink escape rejected",
            "foreign allocation ownership",
            "implicit adoption is refused",
            "allocation provenance mismatch",
            "implicit overwrite is refused",
        ]
        .iter()
        .any(|reason| record.guidance == Some(guidance(reason))),
    };
    if !guidance_valid {
        return Err(Error::InvalidRequest);
    }
    Ok(())
}

pub(super) fn validate_set(records: &[Record], capacity: usize) -> Result<(), Error> {
    if records.len() > capacity {
        return Err(Error::CapacityExceeded);
    }
    let mut operations = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for record in records {
        validate(record).map_err(|_| Error::PersistenceFailed)?;
        if record.descriptor.identity.value() > capacity as u64 {
            return Err(Error::CapacityExceeded);
        }
        if !operations.insert((&record.context.instance_id, &record.operation_id))
            || !identities.insert(record.descriptor.identity)
        {
            return Err(Error::PersistenceFailed);
        }
    }
    Ok(())
}

pub(super) fn replay_matches(
    record: &Record,
    context: &super::SaveProfileContext,
    operation_id: &str,
    profile_id: &str,
) -> bool {
    record.context.same_fence(context)
        && record.operation_id == operation_id
        && record.launch_profile.profile_id == profile_id
        && validate(record).is_ok()
}

pub(super) fn encode(record: &Record) -> Result<Vec<u8>, Error> {
    validate(record)?;
    let bytes = serde_json::to_vec(record).map_err(|_| Error::PersistenceFailed)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(Error::CapacityExceeded);
    }
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> Result<Record, Error> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(Error::PersistenceFailed);
    }
    let record: Record = serde_json::from_slice(bytes).map_err(|_| Error::PersistenceFailed)?;
    // The existing writer already emitted this exact representation. This also refuses duplicate
    // and unknown nested fields without changing valid historical storage bytes.
    if encode(&record).map_err(|_| Error::PersistenceFailed)? != bytes {
        return Err(Error::PersistenceFailed);
    }
    Ok(record)
}

pub(super) fn validate_update(previous: &Record, next: &Record) -> Result<(), Error> {
    validate(next)?;
    if previous.context != next.context
        || previous.operation_id != next.operation_id
        || previous.launch_profile != next.launch_profile
        || previous.descriptor.identity != next.descriptor.identity
        || previous.descriptor.provenance != next.descriptor.provenance
        || (previous.descriptor.baseline.is_some()
            && previous.descriptor.baseline != next.descriptor.baseline)
        || (next.status == Status::Pending && previous != next)
        || (previous.status == Status::Blocked && previous != next)
    {
        return Err(Error::OperationConflict);
    }
    Ok(())
}
