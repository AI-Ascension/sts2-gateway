// SPDX-License-Identifier: MIT

use super::*;
use sts2_gateway::{
    SaveProfileLedgerError, SaveProfileResult, SaveProfileStatus, UserDataProvisioningError,
    UserDataProvisioningOutcome, UserDataProvisioningStatus,
};

pub(super) fn result_response(result: SaveProfileResult) -> (u16, Vec<u8>) {
    let status = match result.status {
        SaveProfileStatus::Unknown => 503,
        SaveProfileStatus::Blocked | SaveProfileStatus::Rejected | SaveProfileStatus::Cancelled => {
            409
        }
        SaveProfileStatus::Accepted | SaveProfileStatus::Settled => 200,
    };
    (
        status,
        json_bytes(&json!({
            "contract": sts2_gateway::SAVE_PROFILE_CONTRACT,
            "operation_id": result.operation_id,
            "route": format!("{:?}", result.route).to_lowercase(),
            "status": format!("{:?}", result.status).to_lowercase(),
            "profile_id": result.profile_id,
            "baseline": result.baseline,
            "user_data": result.user_data,
            "guidance": result.guidance,
            "downstream": result.body,
        })),
    )
}

pub(super) fn provisioning_outcome(outcome: UserDataProvisioningOutcome) -> (u16, Vec<u8>) {
    let http_status = match outcome.status {
        UserDataProvisioningStatus::Unknown => 503,
        UserDataProvisioningStatus::Blocked | UserDataProvisioningStatus::Rejected => 409,
        UserDataProvisioningStatus::Pending => 202,
        UserDataProvisioningStatus::Created => 200,
    };
    (
        http_status,
        json_bytes(&json!({
            "contract": sts2_gateway::SAVE_PROFILE_CONTRACT,
            "operation_id": outcome.operation_id,
            "status": format!("{:?}", outcome.status).to_lowercase(),
            "user_data": outcome.descriptor,
            "guidance": outcome.guidance,
        })),
    )
}

pub(super) fn provisioning_error(error: UserDataProvisioningError) -> (u16, Vec<u8>) {
    let status = match error {
        UserDataProvisioningError::InvalidRequest => 400,
        UserDataProvisioningError::OperationNotFound => 404,
        UserDataProvisioningError::CapacityExceeded => 429,
        UserDataProvisioningError::OperationConflict => 409,
        UserDataProvisioningError::StaleContext => 409,
        UserDataProvisioningError::PersistenceFailed => 503,
        UserDataProvisioningError::Port(_) => 503,
    };
    let code = match error {
        UserDataProvisioningError::OperationConflict => "save_profile_operation_conflict",
        UserDataProvisioningError::CapacityExceeded => "save_profile_capacity_exceeded",
        UserDataProvisioningError::OperationNotFound => "save_profile_operation_not_found",
        UserDataProvisioningError::StaleContext => "save_profile_fence_rejected",
        UserDataProvisioningError::InvalidRequest => "save_profile_request_invalid",
        UserDataProvisioningError::PersistenceFailed => "save_profile_persistence_failed",
        UserDataProvisioningError::Port(_) => "save_profile_provisioning_failed",
    };
    (status, json_error(code))
}

pub(super) fn ledger_error(error: SaveProfileLedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        SaveProfileLedgerError::Fence(_) => (409, "save_profile_fence_rejected"),
        SaveProfileLedgerError::ActiveRun => (409, "save_profile_active_run"),
        SaveProfileLedgerError::BaselineRequired => (409, "save_profile_baseline_required"),
        SaveProfileLedgerError::OperationConflict => (409, "save_profile_operation_conflict"),
        SaveProfileLedgerError::OperationNotFound => (404, "save_profile_operation_not_found"),
        SaveProfileLedgerError::OperationInProgress => (409, "save_profile_operation_in_progress"),
        SaveProfileLedgerError::DuplicateSelection => (409, "save_profile_duplicate_selection"),
        SaveProfileLedgerError::CapacityExceeded => (429, "save_profile_operation_capacity"),
        SaveProfileLedgerError::PersistenceFailed => (503, "save_profile_persistence_failed"),
        SaveProfileLedgerError::ResponseInvalid => (502, "save_profile_response_invalid"),
        SaveProfileLedgerError::Transport(_) => (503, "save_profile_downstream_unavailable"),
        SaveProfileLedgerError::InvalidRequest => (400, "save_profile_request_invalid"),
    };
    (status, json_error(code))
}
