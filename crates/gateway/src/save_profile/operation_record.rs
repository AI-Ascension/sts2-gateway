// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::guidance::RecoveryGuidance;
use super::ledger_response::validate_response;
use super::ledger_types::*;
use super::ledger_validation::{same_operation_request, validate_request};
use super::route::SaveProfileRoute;
use super::types::*;

pub(super) const MAX_RECORD_BYTES: usize = 160 * 1024;

// Private persistence representation. No serialization API is added to operation wire types.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u8,
    operation_id: String,
    #[serde(with = "Request")]
    request: SaveProfileForwardRequest,
    #[serde(with = "Status")]
    status: SaveProfileStatus,
    result: Option<StoredResult>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredResult {
    #[serde(with = "ResultRecord")]
    value: SaveProfileResult,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SaveProfileForwardRequest", deny_unknown_fields)]
struct Request {
    operation_id: String,
    context: SaveProfileContext,
    #[serde(with = "Route")]
    route: SaveProfileRoute,
    #[serde(with = "Operation")]
    operation: SaveProfileOperation,
    body: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SaveProfileOperation", deny_unknown_fields)]
enum Operation {
    List,
    Current,
    Select {
        profile_id: SaveProfileId,
        baseline: ProfileBaseline,
    },
    CreateDisposable {
        launch_profile: LaunchProfileBinding,
        user_data: UserDataDescriptor,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SaveProfileRoute")]
enum Route {
    List,
    Current,
    Select,
    CreateDisposable,
    Lookup,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SaveProfileStatus")]
enum Status {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Blocked,
    Cancelled,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SaveProfileResult", deny_unknown_fields)]
struct ResultRecord {
    operation_id: String,
    #[serde(with = "Route")]
    route: SaveProfileRoute,
    #[serde(with = "Status")]
    status: SaveProfileStatus,
    body: Vec<u8>,
    profile_id: Option<SaveProfileId>,
    baseline: Option<ProfileBaseline>,
    user_data: Option<UserDataDescriptor>,
    guidance: Option<RecoveryGuidance>,
}

pub(super) fn validate(record: &SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
    validate_request(&record.request)?;
    if record.operation_id != record.request.operation_id {
        return Err(SaveProfileLedgerError::InvalidRequest);
    }
    let Some(result) = &record.result else {
        return if record.status == SaveProfileStatus::Accepted {
            Ok(())
        } else {
            Err(SaveProfileLedgerError::InvalidRequest)
        };
    };
    if result.status != record.status
        || result.guidance != RecoveryGuidance::for_status(record.status)
        || result
            .profile_id
            .as_ref()
            .is_some_and(|id| SaveProfileId::try_new(id.as_str()).is_err())
    {
        return Err(SaveProfileLedgerError::ResponseInvalid);
    }
    validate_response(
        &record.request,
        SaveProfileForwardResponse {
            operation_id: result.operation_id.clone(),
            context: record.request.context.clone(),
            route: result.route,
            status: result.status,
            body: result.body.clone(),
            profile_id: result.profile_id.clone(),
            baseline: result.baseline.clone(),
            user_data: result.user_data.clone(),
            reason: None,
        },
    )?;
    Ok(())
}

pub(super) fn encode(
    record: &SaveProfileOperationRecord,
) -> Result<Vec<u8>, SaveProfileLedgerError> {
    validate(record)?;
    let stored = Stored {
        version: 1,
        operation_id: record.operation_id.clone(),
        request: record.request.clone(),
        status: record.status,
        result: record.result.clone().map(|value| StoredResult { value }),
    };
    let bytes =
        serde_json::to_vec(&stored).map_err(|_| SaveProfileLedgerError::PersistenceFailed)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(SaveProfileLedgerError::CapacityExceeded);
    }
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> Result<SaveProfileOperationRecord, SaveProfileLedgerError> {
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(SaveProfileLedgerError::PersistenceFailed);
    }
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|_| SaveProfileLedgerError::PersistenceFailed)?;
    if stored.version != 1 {
        return Err(SaveProfileLedgerError::PersistenceFailed);
    }
    let record = SaveProfileOperationRecord {
        operation_id: stored.operation_id,
        request: stored.request,
        status: stored.status,
        result: stored.result.map(|result| result.value),
    };
    // Exact writer bytes reject duplicate/unknown nested fields and noncanonical spellings.
    if encode(&record)? != bytes {
        return Err(SaveProfileLedgerError::PersistenceFailed);
    }
    Ok(record)
}

pub(super) fn validate_update(
    previous: &SaveProfileOperationRecord,
    next: &SaveProfileOperationRecord,
) -> Result<(), SaveProfileLedgerError> {
    if !same_operation_request(&previous.request, &next.request)
        || previous.request.context != next.request.context
        || (!matches!(
            previous.status,
            SaveProfileStatus::Accepted | SaveProfileStatus::Unknown
        ) && previous != next)
        || next.result.is_none()
    {
        return Err(SaveProfileLedgerError::OperationConflict);
    }
    Ok(())
}
