// SPDX-License-Identifier: MIT

use super::guidance::RecoveryGuidance;
pub(super) use super::provisioning_descriptor::descriptor;
use super::provisioning_types::{
    UserDataProvisioningError, UserDataProvisioningOutcome, UserDataProvisioningRecord,
    UserDataProvisioningStatus,
};
use super::types::{
    LaunchProfileBinding, SaveProfileContext, SaveProfileId, UserDataDescriptor, UserDataIdentity,
};

pub(super) fn validate_request(
    context: &SaveProfileContext,
    operation_id: &str,
    profile_id: &str,
) -> Result<(), UserDataProvisioningError> {
    context
        .validate()
        .map_err(|_| UserDataProvisioningError::InvalidRequest)?;
    validate_operation_id(operation_id)?;
    LaunchProfileBinding::try_new(profile_id.to_owned(), UserDataIdentity::new(1))
        .and_then(|binding| binding.validate())
        .map_err(|_| UserDataProvisioningError::InvalidRequest)
}

pub(super) fn validate_operation_id(operation_id: &str) -> Result<(), UserDataProvisioningError> {
    if operation_id.is_empty()
        || operation_id.len() > super::types::SAVE_PROFILE_MAX_OPERATION_BYTES
        || SaveProfileId::try_new(operation_id).is_err()
    {
        return Err(UserDataProvisioningError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn update_record(
    mut record: UserDataProvisioningRecord,
    status: UserDataProvisioningStatus,
    descriptor: Option<UserDataDescriptor>,
    guidance: Option<RecoveryGuidance>,
) -> UserDataProvisioningRecord {
    if let Some(descriptor) = descriptor {
        record.descriptor = descriptor;
    }
    record.status = status;
    record.guidance = guidance;
    record
}

pub(super) fn outcome(record: &UserDataProvisioningRecord) -> UserDataProvisioningOutcome {
    UserDataProvisioningOutcome {
        operation_id: record.operation_id.clone(),
        status: record.status,
        descriptor: (record.status == UserDataProvisioningStatus::Created)
            .then(|| record.descriptor.clone()),
        guidance: record.guidance.clone(),
    }
}

pub(super) fn blocked(reason: &str) -> (UserDataProvisioningStatus, Option<RecoveryGuidance>) {
    (UserDataProvisioningStatus::Blocked, Some(guidance(reason)))
}

pub(super) fn blocked_record(
    reason: &str,
) -> (
    UserDataProvisioningStatus,
    Option<UserDataDescriptor>,
    Option<RecoveryGuidance>,
) {
    let (status, guidance) = blocked(reason);
    (status, None, guidance)
}

pub(super) fn guidance(reason: &str) -> RecoveryGuidance {
    RecoveryGuidance {
        code: String::from("save_profile_operator_intervention_required"),
        action: format!("{reason}; inspect the isolated allocation before retrying"),
    }
}

pub(super) fn unknown_guidance() -> RecoveryGuidance {
    RecoveryGuidance {
        code: String::from("save_profile_reconcile_required"),
        action: String::from("lookup the original allocation operation before retrying"),
    }
}
