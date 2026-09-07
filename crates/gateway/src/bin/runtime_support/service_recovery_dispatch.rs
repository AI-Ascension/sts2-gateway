// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::{
    RecoveryLeaseProof, RecoveryOperation, RecoveryOperationState, RecoveryUncertaintyReason,
};

use super::super::recovery_frame::{
    RecoveryFrame, RecoveryKind, request_frame, response_frame, response_result,
};
use super::recovery_dispatch_host::host_operation_identity_matches;
use super::{RuntimeService, json_error};

impl RuntimeService {
    /// Persisted gateway dispatches cross the host recovery-control boundary. The intent frame is
    /// sent first so the host can durably retain the complete operation before it receives the
    /// operation reference used for dispatch.
    pub(super) fn recovery_ops_dispatch(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(proof) = self.lease_proof_from_wire(&frame.payload()["lease"]) else {
            return (409, json_error("recovery_stale_lease"));
        };
        let Some((operation_id, payload_digest, original)) =
            super::recovery_ops::parse_operation_ref(&frame.payload()["operation"])
        else {
            return (400, json_error("recovery_operation_ref_invalid"));
        };
        if !super::recovery_ops::context_matches_proof(&original, &proof) {
            return (409, json_error("recovery_operation_context_mismatch"));
        }
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let operation =
            match store.lookup_operation(&proof.instance_id, &operation_id, &payload_digest) {
                Ok(Some(operation)) => operation,
                Ok(None) => return (404, json_error("recovery_operation_not_found")),
                Err(error) => return super::recovery_wire::recovery_store_error(error),
            };
        if !super::recovery_ops::operation_context_matches(&operation, &proof) {
            return (409, json_error("recovery_operation_context_mismatch"));
        }
        let (operation, should_forward) =
            if operation.state == RecoveryOperationState::IntentRecorded {
                match store.mark_dispatched(&proof, &proof.instance_id, &operation_id, now) {
                    Ok(operation) => (operation, true),
                    Err(error) => return super::recovery_wire::recovery_store_error(error),
                }
            } else {
                (operation, false)
            };
        let _ = store;
        if !should_forward {
            return self.recovery_operation_replay(&operation, frame.correlation());
        }
        self.forward_recovery_operation(&proof, &operation, frame.correlation(), frame.auth_proof())
    }

    /// For a Runtime-v3 dispatch there is no recovery-frame proof in the legacy request. The
    /// configured recovery bootstrap secret is used for the host's operation-submit proof; when
    /// it is absent the operation remains UNKNOWN after the durable dispatch marker.
    pub(super) fn forward_recovery_operation(
        &mut self,
        proof: &RecoveryLeaseProof,
        operation: &RecoveryOperation,
        correlation: &str,
        _caller_proof: Option<&str>,
    ) -> (u16, Vec<u8>) {
        let Some(lease) = self.recovery_lease.clone() else {
            return self.recovery_operation_unknown(
                proof,
                operation,
                correlation,
                RecoveryUncertaintyReason::TransportLost,
            );
        };
        // The inbound frame proof authenticates the caller-to-gateway hop. It
        // must never be forwarded as a host credential; derive the submit proof
        // from the gateway's own bootstrap secret and the exact operation.
        let auth_proof = self.recovery_operation_submit_proof(&lease, operation);
        let Some(auth_proof) = auth_proof else {
            return self.recovery_operation_unknown(
                proof,
                operation,
                correlation,
                RecoveryUncertaintyReason::TransportLost,
            );
        };
        let intent_payload = json!({
            "lease": self.recovery_lease_value(&lease),
            "operation": self.recovery_operation_intent_value(operation),
        });
        let intent_frame = request_frame(
            RecoveryKind::OperationIntent,
            &self.config.caller_id,
            correlation,
            Some(&auth_proof),
            intent_payload,
        );
        let intent =
            match self.forward_recovery_control(RecoveryKind::OperationIntent, &intent_frame) {
                Ok(value) => value,
                Err((status, reason)) => {
                    return self.recovery_operation_unknown_with_status(
                        proof,
                        operation,
                        correlation,
                        status,
                        reason,
                    );
                }
            };
        if !host_operation_identity_matches(&intent["payload"]["operation"], operation)
            || !matches!(
                intent["payload"]["result"]["status"].as_str(),
                Some("INTENT_RECORDED" | "DUPLICATE")
            )
        {
            return self.recovery_operation_unknown_with_status(
                proof,
                operation,
                correlation,
                409,
                "host_intent_rejected",
            );
        }
        let dispatch_payload = json!({
            "lease": self.recovery_lease_value(&lease),
            "operation": self.recovery_operation_ref_value(operation),
        });
        let dispatch_frame = request_frame(
            RecoveryKind::OperationDispatch,
            &self.config.caller_id,
            correlation,
            Some(&auth_proof),
            dispatch_payload,
        );
        let dispatch =
            match self.forward_recovery_control(RecoveryKind::OperationDispatch, &dispatch_frame) {
                Ok(value) => value,
                Err((status, reason)) => {
                    return self.recovery_operation_unknown_with_status(
                        proof,
                        operation,
                        correlation,
                        status,
                        reason,
                    );
                }
            };
        self.apply_host_operation_response(proof, operation, correlation, dispatch)
    }

    pub(super) fn recovery_operation_replay(
        &self,
        operation: &RecoveryOperation,
        correlation: &str,
    ) -> (u16, Vec<u8>) {
        let unresolved = matches!(
            operation.state,
            RecoveryOperationState::IntentRecorded
                | RecoveryOperationState::MayHaveBeenDispatched
                | RecoveryOperationState::Accepted
                | RecoveryOperationState::Unknown
        );
        let body = json!({
            "result": response_result(
                super::recovery_payload::operation_state_name(operation.state),
                unresolved,
                unresolved.then_some(1),
            ),
            "operation": self.recovery_operation_value(operation),
        });
        (
            if unresolved { 503 } else { 200 },
            response_frame(
                RecoveryKind::OperationDispatch,
                correlation,
                &self.config.caller_id,
                body,
            ),
        )
    }

    fn recovery_operation_unknown(
        &mut self,
        proof: &RecoveryLeaseProof,
        operation: &RecoveryOperation,
        correlation: &str,
        reason: RecoveryUncertaintyReason,
    ) -> (u16, Vec<u8>) {
        self.recovery_operation_unknown_with_status(
            proof,
            operation,
            correlation,
            503,
            super::recovery_payload::uncertainty_name(reason),
        )
    }

    pub(super) fn recovery_operation_unknown_with_status(
        &mut self,
        _proof: &RecoveryLeaseProof,
        operation: &RecoveryOperation,
        correlation: &str,
        status: u16,
        _reason: &str,
    ) -> (u16, Vec<u8>) {
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let stored = match store.record_outcome(
            &operation.instance_id,
            &operation.operation_id,
            RecoveryOperationState::Unknown,
            None,
            None,
            None,
            Some(RecoveryUncertaintyReason::ReceiptMissing),
            now,
        ) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let body = json!({
            "result": response_result("UNKNOWN", true, Some(1)),
            "operation": self.recovery_operation_value(&stored),
        });
        (
            status.max(503),
            response_frame(
                RecoveryKind::OperationDispatch,
                correlation,
                &self.config.caller_id,
                body,
            ),
        )
    }
}
