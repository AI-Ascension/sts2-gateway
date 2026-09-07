// SPDX-License-Identifier: MIT

use base64::Engine;
use serde_json::{Value, json};
use sts2_gateway::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryAdmissionTicket, RecoveryEffectWitness, RecoveryIntentResult,
    RecoveryLeaseProof, RecoveryOperation, RecoveryOperationIntent, RecoveryOperationState,
    RecoveryStoreError, RecoveryUncertaintyReason, canonicalize_recovery_action, sha256_hex,
};

use super::super::recovery_frame::{
    RecoveryFrame, RecoveryKind, request_frame, response_frame, response_result,
};
use super::RuntimeService;
use super::json_error;

impl RuntimeService {
    pub(super) fn recovery_ops_intent(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(proof) = self.lease_proof_from_wire(&frame.payload()["lease"]) else {
            return (409, json_error("recovery_stale_lease"));
        };
        let Some((operation_id, payload_digest, original, expected, action)) =
            parse_operation_payload(&frame.payload()["operation"])
        else {
            return (400, json_error("recovery_operation_invalid"));
        };
        if !context_matches_proof(&original, &proof) {
            return (409, json_error("recovery_operation_context_mismatch"));
        }
        let intent = RecoveryOperationIntent {
            operation_id,
            deployment_id: proof.deployment_id.clone(),
            instance_id: proof.instance_id.clone(),
            instance_incarnation: proof.instance_incarnation.clone(),
            boot_id: proof.boot_id.clone(),
            authority_generation: proof.authority_generation,
            lease_id: proof.lease_id.clone(),
            lease_epoch: proof.lease_epoch,
            schema_digest: action.0,
            canonical_json: action.1,
            payload_digest,
            expected_state_id: expected.0,
            expected_generation: expected.1,
            catalog_digest: expected.2,
            now_millis: self.recovery_now_millis(),
        };
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let result = match store.record_intent(&proof, intent) {
            Ok(result) => result,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let (status, operation) = match result {
            RecoveryIntentResult::Created(operation) => ("INTENT_RECORDED", operation),
            RecoveryIntentResult::Duplicate(operation) => ("DUPLICATE", operation),
        };
        let body = json!({
            "result": response_result(status, false, None),
            "operation": self.recovery_operation_value(&operation),
        });
        (
            200,
            response_frame(
                RecoveryKind::OperationIntent,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }

    pub(super) fn recovery_ops_lookup(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some((operation_id, payload_digest, original)) =
            parse_operation_ref(&frame.payload()["operation"])
        else {
            return (400, json_error("recovery_operation_ref_invalid"));
        };
        if frame.payload()["lookup_scope"].as_str() != Some("historical_read") {
            return (400, json_error("recovery_lookup_scope_invalid"));
        }
        let Some(store) = self.recovery.as_ref() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let Some(instance_id) = original["instance_id"].as_str() else {
            return (400, json_error("recovery_operation_ref_invalid"));
        };
        let operation = match store.lookup_operation(instance_id, &operation_id, &payload_digest) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let Some(operation) = operation else {
            return self.recovery_ops_not_found(RecoveryKind::OperationLookup, frame.correlation());
        };
        if !operation_ref_matches(&original, &operation) {
            return (409, json_error("recovery_operation_context_mismatch"));
        };
        let Some(proof) = self.recovery_historical_read_proof(&operation) else {
            return self.recovery_ops_unknown(
                RecoveryKind::OperationLookup,
                &operation,
                frame.correlation(),
                503,
                "recovery_auth_unavailable",
            );
        };
        let host_frame = request_frame(
            RecoveryKind::OperationLookup,
            &self.config.caller_id,
            frame.correlation(),
            Some(&proof),
            json!({
                "operation": self.recovery_operation_ref_value(&operation),
                "lookup_scope": "historical_read",
            }),
        );
        let (host_status, host_response) = match self
            .forward_recovery_control_response(RecoveryKind::OperationLookup, &host_frame)
        {
            Ok(response) => response,
            Err((status, reason)) => {
                return self.recovery_ops_unknown(
                    RecoveryKind::OperationLookup,
                    &operation,
                    frame.correlation(),
                    status,
                    reason,
                );
            }
        };
        if let Err(reason) = validate_host_lookup_response(&host_response, &operation) {
            return self.recovery_ops_unknown(
                RecoveryKind::OperationLookup,
                &operation,
                frame.correlation(),
                502,
                reason,
            );
        }
        if host_response["payload"]["result"]["status"].as_str() == Some("NOT_FOUND") {
            return self.recovery_ops_not_found(RecoveryKind::OperationLookup, frame.correlation());
        }
        let evidence = match parse_host_evidence(&host_response, &operation, false) {
            Ok(evidence) => evidence,
            Err(reason) => {
                return self.recovery_ops_unknown(
                    RecoveryKind::OperationLookup,
                    &operation,
                    frame.correlation(),
                    502,
                    reason,
                );
            }
        };
        let stored = match self.persist_host_evidence(
            &operation,
            &evidence,
            host_status,
            &host_response,
            false,
        ) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        self.recovery_ops_response(RecoveryKind::OperationLookup, frame.correlation(), &stored)
    }

    pub(super) fn recovery_ops_reconcile(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some((operation_id, payload_digest, original)) =
            parse_operation_ref(&frame.payload()["operation"])
        else {
            return (400, json_error("recovery_operation_ref_invalid"));
        };
        let Some(fence) = self.current_fence_from_wire(&frame.payload()["current_fence"]) else {
            return (409, json_error("recovery_stale_fence"));
        };
        let Some(strategy) = frame.payload()["strategy"].as_str() else {
            return (400, json_error("recovery_strategy_invalid"));
        };
        if !matches!(strategy, "reobserve" | "receipt_lookup" | "quarantine") {
            return (400, json_error("recovery_strategy_invalid"));
        }
        let Some(store) = self.recovery.as_ref() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let operation = match store.lookup_operation(
            original["instance_id"].as_str().unwrap_or_default(),
            &operation_id,
            &payload_digest,
        ) {
            Ok(Some(operation)) => operation,
            Ok(None) => {
                return self
                    .recovery_ops_not_found(RecoveryKind::OperationReconcile, frame.correlation());
            }
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        if !operation_ref_matches(&original, &operation) {
            return (409, json_error("recovery_operation_context_mismatch"));
        }
        let Some(proof) = self.recovery_reconcile_proof(&operation, strategy, &fence) else {
            return self.recovery_ops_unknown(
                RecoveryKind::OperationReconcile,
                &operation,
                frame.correlation(),
                503,
                "recovery_auth_unavailable",
            );
        };
        let host_frame = request_frame(
            RecoveryKind::OperationReconcile,
            &self.config.caller_id,
            frame.correlation(),
            Some(&proof),
            json!({
                "operation": self.recovery_operation_ref_value(&operation),
                "strategy": strategy,
                "current_fence": super::recovery_wire::fence_value(&fence),
            }),
        );
        let (host_status, host_response) = match self
            .forward_recovery_control_response(RecoveryKind::OperationReconcile, &host_frame)
        {
            Ok(response) => response,
            Err((status, reason)) => {
                return self.recovery_ops_unknown(
                    RecoveryKind::OperationReconcile,
                    &operation,
                    frame.correlation(),
                    status,
                    reason,
                );
            }
        };
        if let Err(reason) = validate_host_reconcile_response(&host_response, &operation) {
            return self.recovery_ops_unknown(
                RecoveryKind::OperationReconcile,
                &operation,
                frame.correlation(),
                502,
                reason,
            );
        }
        if host_response["payload"]["result"]["status"].as_str() == Some("NOT_FOUND") {
            return self
                .recovery_ops_not_found(RecoveryKind::OperationReconcile, frame.correlation());
        }
        let evidence = match parse_host_evidence(&host_response, &operation, true) {
            Ok(evidence) => evidence,
            Err(reason) => {
                return self.recovery_ops_unknown(
                    RecoveryKind::OperationReconcile,
                    &operation,
                    frame.correlation(),
                    502,
                    reason,
                );
            }
        };
        let stored = match self.persist_host_evidence(
            &operation,
            &evidence,
            host_status,
            &host_response,
            true,
        ) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        self.recovery_ops_response(
            RecoveryKind::OperationReconcile,
            frame.correlation(),
            &stored,
        )
    }
}
