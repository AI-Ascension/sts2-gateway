// SPDX-License-Identifier: MIT

use base64::Engine;
use serde_json::{Value, json};
use sts2_gateway::{
    RecoveryEffectWitness, RecoveryLease, RecoveryOperation, RecoveryOperationState,
    RecoveryTicketState, RecoveryUncertaintyReason,
};

use super::RuntimeService;

impl RuntimeService {
    pub(super) fn recovery_operation_ref_value(&self, operation: &RecoveryOperation) -> Value {
        json!({
            "operation_id": operation.operation_id,
            "payload_digest": operation.payload_digest,
            "original_context": {
                "deployment_id": operation.deployment_id,
                "instance_id": operation.instance_id,
                "instance_incarnation": operation.instance_incarnation,
                "boot_id": operation.boot_id,
                "authority_generation": operation.authority_generation,
                "lease_id": operation.lease_id,
                "lease_epoch": operation.lease_epoch,
            },
        })
    }

    pub(super) fn recovery_operation_intent_value(&self, operation: &RecoveryOperation) -> Value {
        let canonical_json_b64 =
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(&operation.canonical_json);
        json!({
            "operation_id": operation.operation_id,
            "payload_digest": operation.payload_digest,
            "original_context": {
                "deployment_id": operation.deployment_id,
                "instance_id": operation.instance_id,
                "instance_incarnation": operation.instance_incarnation,
                "boot_id": operation.boot_id,
                "authority_generation": operation.authority_generation,
                "lease_id": operation.lease_id,
                "lease_epoch": operation.lease_epoch,
            },
            "expected_boundary": {
                "state_id": operation.expected_state_id,
                "generation": operation.expected_generation,
                "catalog_digest": operation.catalog_digest,
            },
            "action": {
                "schema_digest": operation.schema_digest,
                "canonical_json_b64": canonical_json_b64,
                "payload_digest": operation.payload_digest,
            },
        })
    }

    pub(super) fn recovery_lease_value(&self, lease: &RecoveryLease) -> Value {
        json!({
            "deployment_id": lease.deployment_id,
            "instance_id": lease.instance_id,
            "instance_incarnation": lease.instance_incarnation,
            "boot_id": lease.boot_id,
            "authority_generation": lease.authority_generation,
            "lease_id": lease.lease_id,
            "lease_epoch": lease.lease_epoch,
            "fence_token": lease.fence_token,
            "issued_at": super::super::recovery_frame::timestamp_from_millis(lease.issued_at_millis),
            "expires_at": super::super::recovery_frame::timestamp_from_millis(lease.expires_at_millis),
            "ttl_seconds": lease.ttl_seconds,
            "renewal_interval_seconds": lease.renewal_interval_seconds,
        })
    }

    pub(super) fn recovery_operation_value(&self, operation: &RecoveryOperation) -> Value {
        let ticket = self.recovery.as_ref().and_then(|store| {
            store
                .admission_ticket_for_operation(&operation.instance_id, &operation.operation_id)
                .ok()
                .flatten()
        });
        json!({
            "operation_id": operation.operation_id,
            "state": operation_state_name(operation.state),
            "payload_digest": operation.payload_digest,
            "original_context": {
                "deployment_id": operation.deployment_id,
                "instance_id": operation.instance_id,
                "instance_incarnation": operation.instance_incarnation,
                "boot_id": operation.boot_id,
                "authority_generation": operation.authority_generation,
                "lease_id": operation.lease_id,
                "lease_epoch": operation.lease_epoch,
            },
            "expected_boundary": {
                "state_id": operation.expected_state_id,
                "generation": operation.expected_generation,
                "catalog_digest": operation.catalog_digest,
            },
            "action": {
                "schema_digest": operation.schema_digest,
                "canonical_json_b64": base64::engine::general_purpose::STANDARD_NO_PAD
                    .encode(&operation.canonical_json),
                "payload_digest": operation.payload_digest,
            },
            "ticket": ticket.as_ref().map(ticket_value).unwrap_or(Value::Null),
            "witness": operation.witness.as_ref().map(witness_value).unwrap_or(Value::Null),
            "uncertainty_reason": operation.uncertainty_reason.map(uncertainty_name),
            "created_at": super::super::recovery_frame::timestamp_from_millis(operation.created_at_millis),
            "updated_at": super::super::recovery_frame::timestamp_from_millis(operation.updated_at_millis),
        })
    }
}

pub(super) fn operation_state_name(state: RecoveryOperationState) -> &'static str {
    match state {
        RecoveryOperationState::IntentRecorded => "INTENT_RECORDED",
        RecoveryOperationState::MayHaveBeenDispatched => "MAY_HAVE_BEEN_DISPATCHED",
        RecoveryOperationState::Accepted => "ACCEPTED",
        RecoveryOperationState::Settled => "SETTLED",
        RecoveryOperationState::Rejected => "REJECTED",
        RecoveryOperationState::Unknown => "UNKNOWN",
        RecoveryOperationState::Reconciled => "RECONCILED",
    }
}

pub(super) fn ticket_value(ticket: &sts2_gateway::RecoveryAdmissionTicket) -> Value {
    json!({
        "ticket_id": ticket.ticket_id,
        "operation_id": ticket.operation_id,
        "payload_digest": ticket.payload_digest,
        "boot_id": ticket.boot_id,
        "instance_incarnation": ticket.instance_incarnation,
        "lease_epoch": ticket.lease_epoch,
        "host_fence_id": ticket.host_fence_id,
        "state": match ticket.state {
            RecoveryTicketState::Issued => "ISSUED",
            RecoveryTicketState::Admitted => "ADMITTED",
            RecoveryTicketState::Executing => "EXECUTING",
            RecoveryTicketState::EffectWitnessRecorded => "EFFECT_WITNESS_RECORDED",
            RecoveryTicketState::Settled => "SETTLED",
            RecoveryTicketState::Rejected => "REJECTED",
            RecoveryTicketState::Unknown => "UNKNOWN",
        },
        "issued_at": super::super::recovery_frame::timestamp_from_millis(ticket.issued_at_millis),
        "expires_at": super::super::recovery_frame::timestamp_from_millis(ticket.expires_at_millis),
    })
}

pub(super) fn witness_value(witness: &RecoveryEffectWitness) -> Value {
    json!({
        "witness_id": witness.witness_id,
        "operation_id": witness.operation_id,
        "payload_digest": witness.payload_digest,
        "boot_id": witness.boot_id,
        "instance_incarnation": witness.instance_incarnation,
        "host_fence_id": witness.host_fence_id,
        "source": witness.source,
        "state_id": witness.state_id,
        "generation": witness.generation,
        "effect_digest": witness.effect_digest,
        "observed_at": super::super::recovery_frame::timestamp_from_millis(witness.observed_at_millis),
    })
}

pub(super) fn uncertainty_name(reason: RecoveryUncertaintyReason) -> &'static str {
    match reason {
        RecoveryUncertaintyReason::TransportLost => "transport_lost",
        RecoveryUncertaintyReason::Timeout => "timeout",
        RecoveryUncertaintyReason::GatewayCrash => "gateway_crash",
        RecoveryUncertaintyReason::HostCrash => "host_crash",
        RecoveryUncertaintyReason::ReceiptMissing => "receipt_missing",
        RecoveryUncertaintyReason::AuthorityRotated => "authority_rotated",
    }
}
