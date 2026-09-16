// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::{RecoveryContinuationOwnerState, RecoveryStoreError};

use super::super::super::super::continuation_owner_adopt::{AdoptFrame, AdoptFrameError};
use super::{HttpRequest, RuntimeService, json_error};
use super::{owner_state_error, read_owner_snapshot};

const ADOPT_PATH: &str = "/v1/recovery/continuation/owner/adopt";
const CAPABILITY: &str = "continuation_owner_adopt";

pub(super) fn handle(service: &mut RuntimeService, request: &HttpRequest) -> (u16, Vec<u8>) {
    if service.recovery.is_none() {
        return (503, json_error("continuation_owner_unavailable"));
    }
    if request
        .headers
        .get("x-sts2-recovery-capability")
        .map(String::as_str)
        != Some(CAPABILITY)
    {
        return (403, json_error("recovery_capability_forbidden"));
    }
    if request.path != ADOPT_PATH
        || request.method != "POST"
        || !request.content_type_is_json()
        || request.body.is_empty()
    {
        return (400, json_error("continuation_owner_adopt_frame_required"));
    }
    let frame = match AdoptFrame::parse(&request.body) {
        Ok(frame) => frame,
        Err(AdoptFrameError::Oversized) => {
            return (413, json_error("continuation_owner_adopt_frame_oversized"));
        }
        Err(AdoptFrameError::Invalid) => {
            return (400, json_error("continuation_owner_adopt_frame_invalid"));
        }
    };
    if frame.principal() != service.config.caller_id {
        return (403, json_error("recovery_principal_forbidden"));
    }

    let live = match read_owner_snapshot(service) {
        Ok(snapshot) => snapshot,
        Err(error) => return super::super::super::recovery_wire::recovery_store_error(error),
    };
    if live.state != RecoveryContinuationOwnerState::Available {
        return owner_state_error(live.state);
    }
    if live.owner.as_ref() != Some(frame.owner())
        || frame.owner().session_id != service.config.session_id
    {
        return (409, json_error("continuation_owner_fence_mismatch"));
    }

    let now = service.recovery_now_millis();
    let adoption = match service
        .recovery
        .as_mut()
        .ok_or(RecoveryStoreError::PersistenceUnavailable)
        .and_then(|store| store.adopt_continuation_owner(frame.operation_id(), frame.owner(), now))
    {
        Ok(adoption) => adoption,
        Err(error) => return super::super::super::recovery_wire::recovery_store_error(error),
    };

    // The service is mutably serialized while this route runs. Recheck readiness
    // after the store transaction so a deadline crossing cannot return authority.
    let current = match read_owner_snapshot(service) {
        Ok(snapshot) => snapshot,
        Err(error) => return super::super::super::recovery_wire::recovery_store_error(error),
    };
    if current.state != RecoveryContinuationOwnerState::Available {
        return owner_state_error(current.state);
    }
    if current.owner.as_ref() != Some(&adoption.owner)
        || adoption.owner != *frame.owner()
        || service.recovery_fence.as_ref() != Some(&adoption.current_fence)
    {
        return (409, json_error("continuation_owner_fence_mismatch"));
    }

    let payload = json!({
        "result": "ADOPTED",
        "claim": super::claim_value(&adoption.claim),
        "owner": super::super::super::super::continuation_owner::owner_value(&adoption.owner),
        "recovery_authority": super::super::super::allocation_context::recovery_authority_from_owner(
            &adoption.owner,
            &adoption.current_fence,
        ),
    });
    (
        200,
        super::super::super::super::continuation_owner_adopt::response_frame(
            frame.correlation(),
            frame.principal(),
            payload,
        ),
    )
}
