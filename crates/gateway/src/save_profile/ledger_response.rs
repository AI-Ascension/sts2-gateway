// SPDX-License-Identifier: MIT

use super::guidance::RecoveryGuidance;
use super::ledger_types::{
    SaveProfileForwardRequest, SaveProfileForwardResponse, SaveProfileLedgerError,
    SaveProfileResult, SaveProfileStatus,
};
use super::ledger_validation::selected_id;
use super::types::SAVE_PROFILE_MAX_BODY_BYTES;

pub(super) fn validate_response(
    request: &SaveProfileForwardRequest,
    response: SaveProfileForwardResponse,
) -> Result<SaveProfileResult, SaveProfileLedgerError> {
    if response.operation_id != request.operation_id
        || response.context != request.context
        || response.route != request.route
        || response.body.len() > SAVE_PROFILE_MAX_BODY_BYTES
    {
        return Err(SaveProfileLedgerError::ResponseInvalid);
    }
    if response
        .baseline
        .as_ref()
        .is_some_and(|baseline| baseline.validate().is_err())
        || response
            .user_data
            .as_ref()
            .is_some_and(|descriptor| descriptor.validate().is_err())
    {
        return Err(SaveProfileLedgerError::ResponseInvalid);
    }
    if response.status == SaveProfileStatus::Settled {
        match request.route {
            super::route::SaveProfileRoute::Select => {
                let expected = selected_id(request)?;
                if response.profile_id.as_ref() != Some(expected) || response.baseline.is_none() {
                    return Err(SaveProfileLedgerError::ResponseInvalid);
                }
            }
            super::route::SaveProfileRoute::CreateDisposable => {
                let Some(user_data) = response.user_data.as_ref() else {
                    return Err(SaveProfileLedgerError::ResponseInvalid);
                };
                if response.baseline.is_none()
                    || user_data.provenance.owner != "gateway"
                    || user_data.provenance.instance_id != request.context.instance_id
                    || user_data.provenance.operation_id != request.operation_id
                {
                    return Err(SaveProfileLedgerError::ResponseInvalid);
                }
            }
            super::route::SaveProfileRoute::List
            | super::route::SaveProfileRoute::Current
            | super::route::SaveProfileRoute::Lookup => {}
        }
    }
    Ok(SaveProfileResult {
        operation_id: response.operation_id,
        route: response.route,
        status: response.status,
        body: response.body,
        profile_id: response.profile_id,
        baseline: response.baseline,
        user_data: response.user_data,
        guidance: RecoveryGuidance::for_status(response.status),
    })
}
