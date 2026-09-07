// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::{
    RecoveryAdmissionTicket, RecoveryEffectWitness, RecoveryHostFence, RecoveryLeaseProof,
    RecoveryOperation, RecoveryOperationState, RecoveryTicketState, RecoveryUncertaintyReason,
};

use super::super::recovery_frame::{RecoveryKind, response_frame, response_result};
use super::{RuntimeService, json_error};

impl RuntimeService {
    pub(super) fn apply_host_operation_response(
        &mut self,
        proof: &RecoveryLeaseProof,
        operation: &RecoveryOperation,
        correlation: &str,
        response: Value,
    ) -> (u16, Vec<u8>) {
        if !host_operation_identity_matches(&response["payload"]["operation"], operation) {
            return self.recovery_operation_unknown_with_status(
                proof,
                operation,
                correlation,
                502,
                "host_operation_identity_mismatch",
            );
        }
        let status = response["payload"]["result"]["status"]
            .as_str()
            .unwrap_or_default();
        let host_operation = &response["payload"]["operation"];
        let Some(current_fence) = self.recovery_fence.clone() else {
            return self.recovery_operation_unknown_with_status(
                proof,
                operation,
                correlation,
                503,
                "recovery_host_fence_required",
            );
        };
        let ticket = parse_host_ticket(
            Some(&host_operation["ticket"]),
            operation,
            proof,
            &current_fence,
        );
        let witness = parse_host_witness(
            Some(&host_operation["witness"]),
            operation,
            proof,
            ticket.as_ref().map(|ticket| ticket.host_fence_id.as_str()),
        );
        let host_state = host_operation["state"].as_str().unwrap_or_default();
        let (state, uncertainty) = match status {
            "SETTLED" if witness.is_some() && ticket.is_some() => {
                (RecoveryOperationState::Settled, None)
            }
            "ACCEPTED" if ticket.is_some() => (RecoveryOperationState::Accepted, None),
            "MAY_HAVE_BEEN_DISPATCHED" if ticket.is_some() => {
                (RecoveryOperationState::MayHaveBeenDispatched, None)
            }
            "DUPLICATE" if host_state == "SETTLED" && witness.is_some() && ticket.is_some() => {
                (RecoveryOperationState::Settled, None)
            }
            "DUPLICATE" if host_state == "ACCEPTED" && ticket.is_some() => {
                (RecoveryOperationState::Accepted, None)
            }
            "DUPLICATE" if host_state == "MAY_HAVE_BEEN_DISPATCHED" && ticket.is_some() => {
                (RecoveryOperationState::MayHaveBeenDispatched, None)
            }
            "REJECTED" => (RecoveryOperationState::Rejected, None),
            _ => (
                RecoveryOperationState::Unknown,
                Some(RecoveryUncertaintyReason::ReceiptMissing),
            ),
        };
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        if let Some(ticket) = ticket
            && let Err(error) = store.record_host_ticket(proof, &current_fence, ticket, now)
        {
            return super::recovery_wire::recovery_store_error(error);
        }
        let stored = match store.record_outcome(
            &operation.instance_id,
            &operation.operation_id,
            state,
            Some(200),
            serde_json::to_vec(&response).ok(),
            witness,
            uncertainty,
            now,
        ) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let body = json!({
            "result": response_result(
                super::recovery_payload::operation_state_name(stored.state),
                stored.state == RecoveryOperationState::Unknown,
                (stored.state == RecoveryOperationState::Unknown).then_some(1),
            ),
            "operation": self.recovery_operation_value(&stored),
        });
        (
            if stored.state == RecoveryOperationState::Unknown {
                503
            } else {
                200
            },
            response_frame(
                RecoveryKind::OperationDispatch,
                correlation,
                &self.config.caller_id,
                body,
            ),
        )
    }
}

pub(super) fn host_operation_identity_matches(
    value: &Value,
    operation: &RecoveryOperation,
) -> bool {
    value["operation_id"].as_str() == Some(operation.operation_id.as_str())
        && value["payload_digest"].as_str() == Some(operation.payload_digest.as_str())
        && value["original_context"]["deployment_id"].as_str()
            == Some(operation.deployment_id.as_str())
        && value["original_context"]["instance_id"].as_str() == Some(operation.instance_id.as_str())
        && value["original_context"]["instance_incarnation"].as_str()
            == Some(operation.instance_incarnation.as_str())
        && value["original_context"]["boot_id"].as_str() == Some(operation.boot_id.as_str())
        && value["original_context"]["authority_generation"].as_u64()
            == Some(operation.authority_generation)
        && value["original_context"]["lease_id"].as_str() == Some(operation.lease_id.as_str())
        && value["original_context"]["lease_epoch"].as_u64() == Some(operation.lease_epoch)
}

pub(super) fn parse_host_ticket(
    value: Option<&Value>,
    operation: &RecoveryOperation,
    proof: &RecoveryLeaseProof,
    fence: &RecoveryHostFence,
) -> Option<RecoveryAdmissionTicket> {
    parse_host_ticket_for_context(
        value,
        operation,
        &proof.boot_id,
        &proof.instance_incarnation,
        proof.lease_epoch,
        Some(fence.host_fence_id.as_str()),
    )
}

pub(super) fn parse_historical_host_ticket(
    value: Option<&Value>,
    operation: &RecoveryOperation,
) -> Option<RecoveryAdmissionTicket> {
    parse_host_ticket_for_context(
        value,
        operation,
        &operation.boot_id,
        &operation.instance_incarnation,
        operation.lease_epoch,
        None,
    )
}

fn parse_host_ticket_for_context(
    value: Option<&Value>,
    operation: &RecoveryOperation,
    boot_id: &str,
    instance_incarnation: &str,
    lease_epoch: u64,
    expected_fence_id: Option<&str>,
) -> Option<RecoveryAdmissionTicket> {
    let value = value?;
    let object = value.as_object()?;
    if object.len() != 10
        || !super::recovery_wire::valid_uuid_v4(value["ticket_id"].as_str().unwrap_or_default())
        || value["operation_id"].as_str() != Some(operation.operation_id.as_str())
        || value["payload_digest"].as_str() != Some(operation.payload_digest.as_str())
        || value["boot_id"].as_str() != Some(boot_id)
        || value["instance_incarnation"].as_str() != Some(instance_incarnation)
        || value["lease_epoch"].as_u64() != Some(lease_epoch)
        || !super::recovery_wire::valid_uuid_v4(value["host_fence_id"].as_str().unwrap_or_default())
        || expected_fence_id
            .is_some_and(|expected| value["host_fence_id"].as_str() != Some(expected))
    {
        return None;
    }
    let state = match value["state"].as_str()? {
        "ISSUED" => RecoveryTicketState::Issued,
        "ADMITTED" => RecoveryTicketState::Admitted,
        "EXECUTING" => RecoveryTicketState::Executing,
        "EFFECT_WITNESS_RECORDED" => RecoveryTicketState::EffectWitnessRecorded,
        "SETTLED" => RecoveryTicketState::Settled,
        "REJECTED" => RecoveryTicketState::Rejected,
        "UNKNOWN" => RecoveryTicketState::Unknown,
        _ => return None,
    };
    let issued_at_millis = parse_timestamp_millis(value["issued_at"].as_str()?)?;
    let expires_at_millis = parse_timestamp_millis(value["expires_at"].as_str()?)?;
    Some(RecoveryAdmissionTicket {
        ticket_id: value["ticket_id"].as_str()?.to_owned(),
        operation_id: operation.operation_id.clone(),
        payload_digest: operation.payload_digest.clone(),
        boot_id: boot_id.to_owned(),
        instance_incarnation: instance_incarnation.to_owned(),
        lease_epoch,
        host_fence_id: value["host_fence_id"].as_str()?.to_owned(),
        state,
        issued_at_millis,
        expires_at_millis,
    })
}

pub(super) fn parse_host_witness(
    value: Option<&Value>,
    operation: &RecoveryOperation,
    proof: &RecoveryLeaseProof,
    ticket_fence_id: Option<&str>,
) -> Option<RecoveryEffectWitness> {
    parse_host_witness_for_context(
        value,
        operation,
        &proof.boot_id,
        &proof.instance_incarnation,
        ticket_fence_id,
    )
}

pub(super) fn parse_historical_host_witness(
    value: Option<&Value>,
    operation: &RecoveryOperation,
    ticket_fence_id: Option<&str>,
) -> Option<RecoveryEffectWitness> {
    parse_host_witness_for_context(
        value,
        operation,
        &operation.boot_id,
        &operation.instance_incarnation,
        ticket_fence_id,
    )
}

fn parse_host_witness_for_context(
    value: Option<&Value>,
    operation: &RecoveryOperation,
    boot_id: &str,
    instance_incarnation: &str,
    ticket_fence_id: Option<&str>,
) -> Option<RecoveryEffectWitness> {
    let value = value?;
    let object = value.as_object()?;
    if object.len() != 11
        || !super::recovery_wire::valid_uuid_v4(value["witness_id"].as_str()?)
        || value["operation_id"].as_str() != Some(operation.operation_id.as_str())
        || value["payload_digest"].as_str() != Some(operation.payload_digest.as_str())
        || value["boot_id"].as_str() != Some(boot_id)
        || value["instance_incarnation"].as_str() != Some(instance_incarnation)
        || ticket_fence_id != value["host_fence_id"].as_str()
        || !matches!(
            value["source"].as_str(),
            Some("host_game_thread" | "host_receipt" | "authoritative_reobserve")
        )
        || !super::recovery_wire::valid_uuid(value["state_id"].as_str()?)
        || value["generation"].as_u64()? <= operation.expected_generation
        || !super::recovery_wire::valid_digest(value["effect_digest"].as_str()?)
        || value["observed_at"].as_str().is_none()
    {
        return None;
    }
    Some(RecoveryEffectWitness {
        witness_id: value["witness_id"].as_str()?.to_owned(),
        operation_id: operation.operation_id.clone(),
        payload_digest: operation.payload_digest.clone(),
        boot_id: boot_id.to_owned(),
        instance_incarnation: instance_incarnation.to_owned(),
        host_fence_id: value["host_fence_id"].as_str()?.to_owned(),
        source: value["source"].as_str()?.to_owned(),
        state_id: value["state_id"].as_str()?.to_owned(),
        generation: value["generation"].as_u64()?,
        effect_digest: value["effect_digest"].as_str()?.to_owned(),
        observed_at_millis: parse_timestamp_millis(value["observed_at"].as_str()?)?,
    })
}
