// SPDX-License-Identifier: MIT

use super::guidance::RecoveryGuidance;
use super::ledger_types::{
    SaveProfileForwardRequest, SaveProfileLedgerError, SaveProfileOperation, SaveProfileResult,
    SaveProfileStatus,
};
use super::route::SaveProfileRoute;
use super::types::{
    LAUNCH_PROFILE_CONTRACT, LAUNCH_PROFILE_ID, SAVE_PROFILE_MAX_BODY_BYTES,
    SAVE_PROFILE_MAX_OPERATION_BYTES, SaveProfileId,
};

pub(super) fn validate_request(
    request: &SaveProfileForwardRequest,
) -> Result<(), SaveProfileLedgerError> {
    request
        .context
        .validate()
        .map_err(|_| SaveProfileLedgerError::InvalidRequest)?;
    if request.operation_id.is_empty()
        || request.operation_id.len() > SAVE_PROFILE_MAX_OPERATION_BYTES
        || SaveProfileId::try_new(request.operation_id.clone()).is_err()
    {
        return Err(SaveProfileLedgerError::InvalidRequest);
    }
    if request.body.len() > SAVE_PROFILE_MAX_BODY_BYTES
        || (request.route.requires_body() && request.body.is_empty())
        || (!request.route.requires_body() && !request.body.is_empty())
    {
        return Err(SaveProfileLedgerError::InvalidRequest);
    }
    if request.operation.route() != request.route {
        return Err(SaveProfileLedgerError::InvalidRequest);
    }
    match &request.operation {
        SaveProfileOperation::List | SaveProfileOperation::Current => {}
        SaveProfileOperation::Select {
            profile_id,
            baseline,
        } => {
            SaveProfileId::try_new(profile_id.as_str())
                .map_err(|_| SaveProfileLedgerError::InvalidRequest)?;
            baseline
                .validate()
                .map_err(|_| SaveProfileLedgerError::BaselineRequired)?;
        }
        SaveProfileOperation::CreateDisposable {
            launch_profile,
            user_data,
        } => {
            launch_profile
                .validate()
                .map_err(|_| SaveProfileLedgerError::InvalidRequest)?;
            user_data
                .validate()
                .map_err(|_| SaveProfileLedgerError::InvalidRequest)?;
            if launch_profile.profile_id != LAUNCH_PROFILE_ID
                || launch_profile.contract != LAUNCH_PROFILE_CONTRACT
                || launch_profile.user_data != user_data.identity
                || user_data.baseline.is_some()
                || user_data.provenance.owner != "gateway"
                || user_data.provenance.instance_id != request.context.instance_id
                || user_data.provenance.operation_id != request.operation_id
                || user_data.provenance.contract != LAUNCH_PROFILE_CONTRACT
            {
                return Err(SaveProfileLedgerError::InvalidRequest);
            }
        }
    }
    Ok(())
}

pub(super) fn same_operation_request(
    left: &SaveProfileForwardRequest,
    right: &SaveProfileForwardRequest,
) -> bool {
    left.operation_id == right.operation_id
        && left.context.same_fence(&right.context)
        && left.route == right.route
        && left.operation == right.operation
        && left.body == right.body
}

pub(super) fn selected_id(
    request: &SaveProfileForwardRequest,
) -> Result<&SaveProfileId, SaveProfileLedgerError> {
    match &request.operation {
        SaveProfileOperation::Select {
            profile_id,
            baseline,
        } => {
            SaveProfileId::try_new(profile_id.as_str())
                .map_err(|_| SaveProfileLedgerError::InvalidRequest)?;
            baseline
                .validate()
                .map_err(|_| SaveProfileLedgerError::BaselineRequired)?;
            Ok(profile_id)
        }
        _ => Err(SaveProfileLedgerError::InvalidRequest),
    }
}

pub(super) fn key(request: &SaveProfileForwardRequest) -> (String, String) {
    (
        request.context.instance_id.clone(),
        request.operation_id.clone(),
    )
}

pub(super) fn unknown_result(request: &SaveProfileForwardRequest) -> SaveProfileResult {
    SaveProfileResult {
        operation_id: request.operation_id.clone(),
        route: request.route,
        status: SaveProfileStatus::Unknown,
        body: Vec::new(),
        profile_id: None,
        baseline: None,
        user_data: None,
        guidance: RecoveryGuidance::for_status(SaveProfileStatus::Unknown),
    }
}

pub(super) fn accepted_result(request: &SaveProfileForwardRequest) -> SaveProfileResult {
    SaveProfileResult {
        operation_id: request.operation_id.clone(),
        route: request.route,
        status: SaveProfileStatus::Accepted,
        body: Vec::new(),
        profile_id: None,
        baseline: None,
        user_data: None,
        guidance: None,
    }
}

pub(super) fn rejected_result(
    request: &SaveProfileForwardRequest,
    status: SaveProfileStatus,
) -> SaveProfileResult {
    SaveProfileResult {
        operation_id: request.operation_id.clone(),
        route: request.route,
        status,
        body: Vec::new(),
        profile_id: None,
        baseline: None,
        user_data: None,
        guidance: RecoveryGuidance::for_status(status),
    }
}
