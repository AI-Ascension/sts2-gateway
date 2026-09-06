// SPDX-License-Identifier: MIT

use super::super::MAX_RECOVERY_RESPONSE_BYTES;
use super::super::recovery_types::{
    RecoveryEffectWitness, RecoveryIntentResult, RecoveryLeaseProof, RecoveryOperation,
    RecoveryOperationIntent, RecoveryOperationState, RecoveryStoreError, RecoveryUncertaintyReason,
    validate_digest, validate_identity, validate_uuid, validate_uuid_v4, validate_wire,
};
use super::GatewayRecoveryStore;
pub(super) use super::operation_helpers::select_operation;
use super::operation_helpers::{
    insert_operation, same_intent, update_operation, valid_transition, validate_intent,
    validate_witness_context, validate_witness_shape,
};

impl GatewayRecoveryStore {
    pub fn record_intent(
        &mut self,
        proof: &RecoveryLeaseProof,
        intent: RecoveryOperationIntent,
    ) -> Result<RecoveryIntentResult, RecoveryStoreError> {
        validate_intent(&intent)?;
        if intent.deployment_id != proof.deployment_id
            || intent.instance_id != proof.instance_id
            || intent.instance_incarnation != proof.instance_incarnation
            || intent.boot_id != proof.boot_id
            || intent.authority_generation != proof.authority_generation
            || intent.lease_id != proof.lease_id
            || intent.lease_epoch != proof.lease_epoch
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        self.ensure_context(proof, intent.now_millis)?;
        let unresolved_capacity = self.config.unresolved_capacity;
        let retained_capacity = self.config.retained_capacity;
        let tx = self.transaction()?;
        let existing = select_operation(&tx, &intent.instance_id, &intent.operation_id)?;
        if let Some(existing) = existing {
            if same_intent(&existing, &intent) {
                return Ok(RecoveryIntentResult::Duplicate(existing));
            }
            return Err(RecoveryStoreError::OperationConflict);
        }
        let unresolved: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM operations
                 WHERE state IN ('INTENT_RECORDED', 'MAY_HAVE_BEEN_DISPATCHED', 'ACCEPTED', 'UNKNOWN')",
                [],
                |row| row.get(0),
            )
            .map_err(super::map_sql_error)?;
        if usize::try_from(unresolved).unwrap_or(usize::MAX) >= unresolved_capacity {
            return Err(RecoveryStoreError::CapacityExceeded);
        }
        let retained: i64 = tx
            .query_row(
                "SELECT (SELECT COUNT(*) FROM operations)
                        + (SELECT COUNT(*) FROM operation_archive)",
                [],
                |row| row.get(0),
            )
            .map_err(super::map_sql_error)?;
        if usize::try_from(retained).unwrap_or(usize::MAX) >= retained_capacity {
            return Err(RecoveryStoreError::CapacityExceeded);
        }
        let operation = RecoveryOperation {
            operation_id: intent.operation_id.clone(),
            deployment_id: intent.deployment_id.clone(),
            instance_id: intent.instance_id.clone(),
            instance_incarnation: intent.instance_incarnation.clone(),
            boot_id: intent.boot_id.clone(),
            authority_generation: intent.authority_generation,
            lease_id: intent.lease_id.clone(),
            lease_epoch: intent.lease_epoch,
            state: RecoveryOperationState::IntentRecorded,
            payload_digest: intent.payload_digest.clone(),
            schema_digest: intent.schema_digest.clone(),
            canonical_json: intent.canonical_json.clone(),
            expected_state_id: intent.expected_state_id.clone(),
            expected_generation: intent.expected_generation,
            catalog_digest: intent.catalog_digest.clone(),
            response_status: None,
            response_body: None,
            witness: None,
            uncertainty_reason: None,
            created_at_millis: intent.now_millis,
            updated_at_millis: intent.now_millis,
        };
        insert_operation(&tx, &operation)?;
        tx.commit().map_err(super::map_sql_error)?;
        Ok(RecoveryIntentResult::Created(operation))
    }

    pub fn mark_dispatched(
        &mut self,
        proof: &RecoveryLeaseProof,
        instance_id: &str,
        operation_id: &str,
        now_millis: u64,
    ) -> Result<RecoveryOperation, RecoveryStoreError> {
        validate_uuid("instance_id", instance_id)?;
        validate_uuid_v4("operation_id", operation_id)?;
        self.ensure_context(proof, now_millis)?;
        let tx = self.transaction()?;
        let operation = select_operation(&tx, instance_id, operation_id)?
            .ok_or(RecoveryStoreError::OperationNotFound)?;
        if operation.boot_id != proof.boot_id
            || operation.instance_incarnation != proof.instance_incarnation
            || operation.authority_generation != proof.authority_generation
            || operation.lease_id != proof.lease_id
            || operation.lease_epoch != proof.lease_epoch
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        if operation.state != RecoveryOperationState::IntentRecorded {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        tx.execute(
            "UPDATE operations SET state = 'MAY_HAVE_BEEN_DISPATCHED', updated_at_millis = ?1
             WHERE instance_id = ?2 AND operation_id = ?3 AND state = 'INTENT_RECORDED'",
            rusqlite::params![now_millis as i64, instance_id, operation_id],
        )
        .map_err(super::map_sql_error)?;
        tx.commit().map_err(super::map_sql_error)?;
        Ok(RecoveryOperation {
            state: RecoveryOperationState::MayHaveBeenDispatched,
            updated_at_millis: now_millis,
            ..operation
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_outcome(
        &mut self,
        instance_id: &str,
        operation_id: &str,
        state: RecoveryOperationState,
        response_status: Option<u16>,
        response_body: Option<Vec<u8>>,
        witness: Option<RecoveryEffectWitness>,
        uncertainty_reason: Option<RecoveryUncertaintyReason>,
        now_millis: u64,
    ) -> Result<RecoveryOperation, RecoveryStoreError> {
        validate_uuid("instance_id", instance_id)?;
        validate_uuid_v4("operation_id", operation_id)?;
        if let Some(body) = response_body.as_ref()
            && body.len() > MAX_RECOVERY_RESPONSE_BYTES
        {
            return Err(RecoveryStoreError::InvalidInput(
                "operation response exceeds the recovery bound".to_owned(),
            ));
        }
        validate_wire(now_millis, "now_millis")?;
        if state == RecoveryOperationState::Unknown && uncertainty_reason.is_none() {
            return Err(RecoveryStoreError::ContractMismatch(
                "unknown operation requires an uncertainty reason".to_owned(),
            ));
        }
        if state != RecoveryOperationState::Unknown && uncertainty_reason.is_some() {
            return Err(RecoveryStoreError::ContractMismatch(
                "uncertainty reason is only valid for unknown operations".to_owned(),
            ));
        }
        validate_witness_shape(operation_id, state, witness.as_ref())?;
        let tx = self.transaction()?;
        let operation = select_operation(&tx, instance_id, operation_id)?
            .ok_or(RecoveryStoreError::OperationNotFound)?;
        validate_witness_context(&operation, witness.as_ref())?;
        if !valid_transition(operation.state, state) {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        let next = RecoveryOperation {
            state,
            response_status,
            response_body,
            witness,
            uncertainty_reason,
            updated_at_millis: now_millis,
            ..operation
        };
        update_operation(&tx, &next)?;
        tx.commit().map_err(super::map_sql_error)?;
        Ok(next)
    }

    pub fn lookup_operation(
        &self,
        instance_id: &str,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<Option<RecoveryOperation>, RecoveryStoreError> {
        validate_identity("instance_id", instance_id)?;
        validate_identity("operation_id", operation_id)?;
        validate_digest("payload_digest", payload_digest)?;
        let operation = select_operation(&self.conn, instance_id, operation_id)?;
        match operation {
            Some(operation) if operation.payload_digest == payload_digest => Ok(Some(operation)),
            Some(_) => Err(RecoveryStoreError::OperationConflict),
            None => Ok(None),
        }
    }

    pub fn reconcile_operation(
        &mut self,
        instance_id: &str,
        operation_id: &str,
        payload_digest: &str,
        witness: Option<RecoveryEffectWitness>,
        now_millis: u64,
    ) -> Result<RecoveryOperation, RecoveryStoreError> {
        let operation = self
            .lookup_operation(instance_id, operation_id, payload_digest)?
            .ok_or(RecoveryStoreError::OperationNotFound)?;
        validate_witness_shape(
            operation_id,
            RecoveryOperationState::Reconciled,
            witness.as_ref(),
        )?;
        let Some(witness) = witness else {
            return Ok(operation);
        };
        validate_witness_context(&operation, Some(&witness))?;
        if operation.state == RecoveryOperationState::Reconciled {
            if operation.witness != Some(witness.clone()) {
                return Err(RecoveryStoreError::OperationConflict);
            }
            return Ok(operation);
        }
        if !operation.state.unresolved() {
            return Err(RecoveryStoreError::InvalidTransition);
        }
        self.record_outcome(
            instance_id,
            operation_id,
            RecoveryOperationState::Reconciled,
            operation.response_status,
            operation.response_body,
            Some(witness),
            None,
            now_millis,
        )
    }

    pub fn unresolved_count(&self) -> Result<usize, RecoveryStoreError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM operations
                 WHERE state IN ('INTENT_RECORDED', 'MAY_HAVE_BEEN_DISPATCHED', 'ACCEPTED', 'UNKNOWN')",
                [],
                |row| row.get(0),
            )
            .map_err(super::map_sql_error)?;
        usize::try_from(count)
            .map_err(|_| RecoveryStoreError::Corrupt("operation count overflow".to_owned()))
    }

    pub fn retained_count(&self) -> Result<usize, RecoveryStoreError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM operations)
                        + (SELECT COUNT(*) FROM operation_archive)",
                [],
                |row| row.get(0),
            )
            .map_err(super::map_sql_error)?;
        usize::try_from(count)
            .map_err(|_| RecoveryStoreError::Corrupt("operation count overflow".to_owned()))
    }
}
