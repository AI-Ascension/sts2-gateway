// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_gateway::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryOperationIntent,
    RecoveryOperationState, canonicalize_recovery_action, sha256_hex,
};

use super::{HttpRequest, RuntimeService, json_error};

impl RuntimeService {
    /// Adapts a frozen Runtime-v3 action into a durable recovery operation and sends the
    /// operation through the host recovery-control mux. The legacy gameplay body is never used as
    /// settlement evidence; only the host's operation record and exact game-thread witness count.
    pub(super) fn recovery_v3_dispatch(
        &mut self,
        request: &HttpRequest,
        _route: super::RuntimeV3GameplayRoute,
        envelope: Value,
    ) -> (u16, Vec<u8>) {
        let Some(lease) = self.recovery_lease.clone() else {
            return (409, json_error("lease_not_active"));
        };
        let Some(_fence) = self.recovery_fence.clone() else {
            return (503, json_error("recovery_host_fence_required"));
        };
        let action = match serde_json::to_vec(&envelope["action"])
            .ok()
            .and_then(|bytes| canonicalize_recovery_action(&bytes).ok())
        {
            Some(action) if action.len() <= sts2_gateway::MAX_RECOVERY_ACTION_BYTES => action,
            _ => return (400, json_error("recovery_action_invalid")),
        };
        let (Some(operation_id), Some(state_id), Some(generation)) = (
            envelope["operation_id"].as_str(),
            envelope["state_id"].as_str(),
            envelope["generation"].as_u64(),
        ) else {
            return (400, json_error("recovery_operation_invalid"));
        };
        if !super::recovery_wire::valid_uuid_v4(operation_id)
            || !super::recovery_wire::valid_uuid(state_id)
        {
            return (400, json_error("recovery_operation_invalid"));
        }
        let payload_digest = sha256_hex(&action);
        let proof = lease.proof();
        let intent = RecoveryOperationIntent {
            operation_id: operation_id.to_owned(),
            deployment_id: proof.deployment_id.clone(),
            instance_id: proof.instance_id.clone(),
            instance_incarnation: proof.instance_incarnation.clone(),
            boot_id: proof.boot_id.clone(),
            authority_generation: proof.authority_generation,
            lease_id: proof.lease_id.clone(),
            lease_epoch: proof.lease_epoch,
            schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
            canonical_json: action,
            payload_digest,
            expected_state_id: state_id.to_owned(),
            expected_generation: generation,
            catalog_digest: self.config.recovery_release.profile_digest.clone(),
            now_millis: self.recovery_now_millis(),
        };
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            .unwrap_or_else(|| envelope["correlation_id"].as_str().unwrap_or_default());
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let (operation, should_dispatch) = match store.record_intent(&proof, intent) {
            Ok(RecoveryIntentResult::Created(operation)) => (operation, true),
            Ok(RecoveryIntentResult::Duplicate(operation)) => {
                if matches!(
                    operation.state,
                    RecoveryOperationState::Settled
                        | RecoveryOperationState::Accepted
                        | RecoveryOperationState::Rejected
                        | RecoveryOperationState::Reconciled
                ) {
                    return self.recovery_operation_replay(&operation, correlation);
                }
                if operation.state != RecoveryOperationState::IntentRecorded {
                    return self.recovery_operation_replay(&operation, correlation);
                }
                (operation, true)
            }
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let _ = store;
        if !should_dispatch {
            return self.recovery_operation_replay(&operation, correlation);
        }
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let operation = match store.mark_dispatched(&proof, &proof.instance_id, operation_id, now) {
            Ok(operation) => operation,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        let _ = store;
        self.forward_recovery_operation(&proof, &operation, correlation, None)
    }
}
