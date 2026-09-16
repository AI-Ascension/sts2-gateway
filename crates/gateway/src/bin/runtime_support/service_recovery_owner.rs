// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::{
    RecoveryContinuationOwner, RecoveryContinuationOwnerClaimResult,
    RecoveryContinuationOwnerSnapshot, RecoveryContinuationOwnerState,
};

use super::super::super::continuation_owner::{
    ContinuationOwnerFrame, ContinuationOwnerFrameError, ContinuationOwnerKind, owner_value,
    response_frame,
};
use super::super::{HttpRequest, RuntimeService, json_error};

#[path = "service_recovery_owner_adopt.rs"]
mod adopt;

pub(super) fn dispatch(
    service: &mut RuntimeService,
    request: &HttpRequest,
) -> Option<(u16, Vec<u8>)> {
    if request.method != "POST" {
        return None;
    }
    let kind = match request.path.as_str() {
        "/v1/recovery/continuation/owner/adopt" => return Some(adopt::handle(service, request)),
        "/v1/recovery/continuation/owner/read" => ContinuationOwnerKind::Read,
        "/v1/recovery/continuation/owner/claim" => ContinuationOwnerKind::Claim,
        "/v1/recovery/continuation/owner/lookup" => ContinuationOwnerKind::Lookup,
        _ => return None,
    };
    Some(handle(service, request, kind))
}

pub(super) fn handle(
    service: &mut RuntimeService,
    request: &HttpRequest,
    kind: ContinuationOwnerKind,
) -> (u16, Vec<u8>) {
    if service.recovery.is_none() {
        return (503, json_error("continuation_owner_unavailable"));
    }
    if request
        .headers
        .get("x-sts2-recovery-capability")
        .map(String::as_str)
        != Some(kind.capability())
    {
        return (403, json_error("recovery_capability_forbidden"));
    }
    if !request.content_type_is_json() || request.body.is_empty() {
        return (400, json_error("continuation_owner_frame_required"));
    }
    let frame = match ContinuationOwnerFrame::parse(&request.body, kind) {
        Ok(frame) => frame,
        Err(ContinuationOwnerFrameError::Oversized) => {
            return (413, json_error("continuation_owner_frame_oversized"));
        }
        Err(ContinuationOwnerFrameError::Invalid) => {
            return (400, json_error("continuation_owner_frame_invalid"));
        }
    };
    if frame.principal() != service.config.caller_id {
        return (403, json_error("recovery_principal_forbidden"));
    }
    match kind {
        ContinuationOwnerKind::Read => read_current_owner(service, &frame),
        ContinuationOwnerKind::Claim => claim_owner(service, &frame),
        ContinuationOwnerKind::Lookup => lookup_claim(service, &frame),
    }
}

fn read_current_owner(
    service: &mut RuntimeService,
    frame: &ContinuationOwnerFrame,
) -> (u16, Vec<u8>) {
    let snapshot = match read_owner_snapshot(service) {
        Ok(snapshot) => snapshot,
        Err(error) => return super::super::recovery_wire::recovery_store_error(error),
    };
    let payload = json!({
        "state": owner_state_name(snapshot.state),
        "owner": snapshot.owner.as_ref().map(owner_value),
    });
    (
        200,
        response_frame(
            ContinuationOwnerKind::Read,
            frame.correlation(),
            &service.config.caller_id,
            payload,
        ),
    )
}

fn claim_owner(service: &mut RuntimeService, frame: &ContinuationOwnerFrame) -> (u16, Vec<u8>) {
    let Some(operation_id) = frame.payload()["operation_id"].as_str() else {
        return (400, json_error("continuation_owner_claim_invalid"));
    };
    let expected_owner: RecoveryContinuationOwner =
        match serde_json::from_value(frame.payload()["expected_owner"].clone()) {
            Ok(owner) => owner,
            Err(_) => return (400, json_error("continuation_owner_claim_invalid")),
        };
    let current = match read_owner_snapshot(service) {
        Ok(snapshot) => snapshot,
        Err(error) => return super::super::recovery_wire::recovery_store_error(error),
    };
    if current.state != RecoveryContinuationOwnerState::Available {
        return owner_state_error(current.state);
    }
    if current.owner.as_ref() != Some(&expected_owner) {
        return (409, json_error("continuation_owner_fence_mismatch"));
    }
    let now = service.recovery_now_millis();
    let result = match service
        .recovery
        .as_mut()
        .ok_or(sts2_gateway::RecoveryStoreError::PersistenceUnavailable)
        .and_then(|store| store.claim_continuation_owner(operation_id, &expected_owner, now))
    {
        Ok(result) => result,
        Err(error) => return super::super::recovery_wire::recovery_store_error(error),
    };
    let (result, claim) = match result {
        RecoveryContinuationOwnerClaimResult::Created(claim) => ("CLAIMED", claim),
        RecoveryContinuationOwnerClaimResult::Duplicate(claim) => ("DUPLICATE", claim),
    };
    let payload = json!({
        "result": result,
        "claim": claim_value(&claim),
    });
    (
        200,
        response_frame(
            ContinuationOwnerKind::Claim,
            frame.correlation(),
            &service.config.caller_id,
            payload,
        ),
    )
}

fn lookup_claim(service: &mut RuntimeService, frame: &ContinuationOwnerFrame) -> (u16, Vec<u8>) {
    let Some(operation_id) = frame.payload()["operation_id"].as_str() else {
        return (400, json_error("continuation_owner_lookup_invalid"));
    };
    let snapshot = match read_owner_snapshot(service) {
        Ok(snapshot) => snapshot,
        Err(error) => return super::super::recovery_wire::recovery_store_error(error),
    };
    let claim = match service
        .recovery
        .as_ref()
        .ok_or(sts2_gateway::RecoveryStoreError::PersistenceUnavailable)
        .and_then(|store| store.lookup_continuation_owner_claim(operation_id))
    {
        Ok(claim) => claim,
        Err(error) => return super::super::recovery_wire::recovery_store_error(error),
    };
    let payload = json!({
        "claim_state": if claim.is_some() { "historical" } else { "not_found" },
        "claim": claim.as_ref().map(claim_value),
        "current_owner": {
            "state": owner_state_name(snapshot.state),
            "owner": snapshot.owner.as_ref().map(owner_value),
        },
    });
    (
        200,
        response_frame(
            ContinuationOwnerKind::Lookup,
            frame.correlation(),
            &service.config.caller_id,
            payload,
        ),
    )
}

fn claim_value(claim: &sts2_gateway::RecoveryContinuationOwnerClaim) -> Value {
    json!({
        "operation_id": claim.operation_id,
        "request_digest": claim.request_digest,
        "owner": owner_value(&claim.owner),
        "claimed_at_millis": claim.claimed_at_millis,
    })
}

pub(super) fn read_owner_snapshot(
    service: &mut RuntimeService,
) -> Result<RecoveryContinuationOwnerSnapshot, sts2_gateway::RecoveryStoreError> {
    service.check_recovery_deadline();
    let now = service.recovery_now_millis();
    let Some(store) = service.recovery.as_ref() else {
        return Err(sts2_gateway::RecoveryStoreError::PersistenceUnavailable);
    };
    let mut snapshot = store.current_continuation_owner(&service.config.session_id, now)?;
    if snapshot.state == RecoveryContinuationOwnerState::Available {
        let memory_owner = service
            .recovery_lease
            .as_ref()
            .zip(service.recovery_fence.as_ref())
            .map(|(lease, fence)| RecoveryContinuationOwner {
                deployment_id: lease.deployment_id.clone(),
                instance_id: lease.instance_id.clone(),
                instance_incarnation: lease.instance_incarnation.clone(),
                boot_id: lease.boot_id.clone(),
                authority_generation: lease.authority_generation,
                host_fence_id: fence.host_fence_id.clone(),
                host_fence_generation: fence.fence_generation,
                lease_id: lease.lease_id.clone(),
                lease_epoch: lease.lease_epoch,
                session_id: service.config.session_id.clone(),
                lease_expires_at_millis: lease.expires_at_millis,
            });
        let boot_matches = snapshot.owner.as_ref().is_some_and(|owner| {
            service.recovery_boot.as_ref().is_some_and(|boot| {
                boot.deployment_id == owner.deployment_id
                    && boot.instance_id == owner.instance_id
                    && boot.instance_incarnation == owner.instance_incarnation
                    && boot.boot_id == owner.boot_id
                    && boot.authority_generation == owner.authority_generation
                    && boot.state == sts2_gateway::RecoveryBootState::Ready
            })
        });
        let grant_matches = service
            .recovery_lease
            .as_ref()
            .map(|lease| service.active_host_grant_matches(lease))
            .transpose()?
            .unwrap_or(false);
        let memory_is_live = service.lease_active
            && !service.lease_revoked
            && !service.shutdown_requested
            && service
                .recovery_lease_deadline
                .is_some_and(|deadline| std::time::Instant::now() < deadline);
        if !memory_is_live
            || !boot_matches
            || !grant_matches
            || memory_owner.as_ref() != snapshot.owner.as_ref()
        {
            snapshot.state = RecoveryContinuationOwnerState::Unknown;
        }
    }
    Ok(snapshot)
}

fn owner_state_name(state: RecoveryContinuationOwnerState) -> &'static str {
    match state {
        RecoveryContinuationOwnerState::Available => "available",
        RecoveryContinuationOwnerState::Absent => "absent",
        RecoveryContinuationOwnerState::Expired => "expired",
        RecoveryContinuationOwnerState::Revoked => "revoked",
        RecoveryContinuationOwnerState::Unknown => "unknown",
    }
}

pub(super) fn owner_state_error(state: RecoveryContinuationOwnerState) -> (u16, Vec<u8>) {
    match state {
        RecoveryContinuationOwnerState::Available => {
            (409, json_error("continuation_owner_invalid"))
        }
        RecoveryContinuationOwnerState::Absent => (404, json_error("continuation_owner_absent")),
        RecoveryContinuationOwnerState::Expired => (410, json_error("continuation_owner_expired")),
        RecoveryContinuationOwnerState::Revoked => (409, json_error("continuation_owner_revoked")),
        RecoveryContinuationOwnerState::Unknown => (503, json_error("continuation_owner_unknown")),
    }
}
