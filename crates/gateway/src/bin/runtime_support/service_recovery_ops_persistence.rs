// SPDX-License-Identifier: MIT

impl RuntimeService {

    fn persist_host_evidence(
        &mut self,
        operation: &RecoveryOperation,
        evidence: &HostOperationEvidence,
        response_status: u16,
        response: &Value,
        reconciliation: bool,
    ) -> Result<RecoveryOperation, RecoveryStoreError> {
        let response_body = serde_json::to_vec(response).ok();
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return Err(RecoveryStoreError::PersistenceUnavailable);
        };
        if let Some(ticket) = evidence.ticket.clone() {
            store.record_historical_host_ticket(&operation.instance_id, ticket)?;
        }
        let state = if evidence.operation_state == RecoveryOperationState::Reconciled
            || (reconciliation && evidence.witness.is_some())
        {
            RecoveryOperationState::Reconciled
        } else {
            match evidence.result_status.as_str() {
                "ACCEPTED" => RecoveryOperationState::Accepted,
                "SETTLED" => RecoveryOperationState::Settled,
                "REJECTED" => RecoveryOperationState::Rejected,
                "UNKNOWN" => RecoveryOperationState::Unknown,
                _ => {
                    return Err(RecoveryStoreError::ContractMismatch(
                        "host recovery result is not an operation outcome".to_owned(),
                    ));
                }
            }
        };
        // An UNKNOWN gateway record can only be closed by a read-only witness. A
        // historical lookup may report SETTLED, but it must not turn an uncertain
        // operation into a receipt-less terminal state (the store intentionally
        // permits UNKNOWN -> RECONCILED only).
        let state = if operation.state == RecoveryOperationState::Unknown
            && state != RecoveryOperationState::Reconciled
        {
            RecoveryOperationState::Unknown
        } else {
            state
        };
        if matches!(
            operation.state,
            RecoveryOperationState::Settled
                | RecoveryOperationState::Rejected
                | RecoveryOperationState::Reconciled
        ) && operation.state != state
        {
            return Ok(operation.clone());
        }
        let uncertainty = (state == RecoveryOperationState::Unknown)
            .then_some(RecoveryUncertaintyReason::ReceiptMissing);
        let witness = evidence.witness.clone();
        store.record_outcome(
            &operation.instance_id,
            &operation.operation_id,
            state,
            Some(response_status),
            response_body,
            witness,
            uncertainty,
            now,
        )
    }

    fn recovery_ops_response(
        &self,
        kind: RecoveryKind,
        correlation: &str,
        operation: &RecoveryOperation,
    ) -> (u16, Vec<u8>) {
        let unresolved = matches!(
            operation.state,
            RecoveryOperationState::IntentRecorded
                | RecoveryOperationState::MayHaveBeenDispatched
                | RecoveryOperationState::Accepted
                | RecoveryOperationState::Unknown
        );
        let result = response_result(
            super::recovery_payload::operation_state_name(operation.state),
            unresolved,
            unresolved.then_some(1),
        );
        let payload = if kind == RecoveryKind::OperationLookup {
            json!({
                "result": result,
                "operation": self.recovery_operation_value(operation),
                "mutation_authorized": false,
            })
        } else {
            json!({
                "result": result,
                "operation": self.recovery_operation_value(operation),
                "witness": operation.witness.as_ref()
                    .map(super::recovery_payload::witness_value)
                    .unwrap_or(Value::Null),
            })
        };
        (
            if unresolved { 503 } else { 200 },
            response_frame(kind, correlation, &self.config.caller_id, payload),
        )
    }

    fn recovery_ops_not_found(&self, kind: RecoveryKind, correlation: &str) -> (u16, Vec<u8>) {
        let mut body = json!({
            "result": response_result("NOT_FOUND", false, None),
            "operation": Value::Null,
        });
        if kind == RecoveryKind::OperationLookup {
            body["mutation_authorized"] = Value::Bool(false);
        } else {
            body["witness"] = Value::Null;
        }
        (
            404,
            response_frame(kind, correlation, &self.config.caller_id, body),
        )
    }

    fn recovery_ops_unknown(
        &mut self,
        kind: RecoveryKind,
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
            Err(error) => {
                if matches!(
                    operation.state,
                    RecoveryOperationState::Settled
                        | RecoveryOperationState::Rejected
                        | RecoveryOperationState::Reconciled
                ) {
                    operation.clone()
                } else {
                    return super::recovery_wire::recovery_store_error(error);
                }
            }
        };
        let unresolved = matches!(
            stored.state,
            RecoveryOperationState::IntentRecorded
                | RecoveryOperationState::MayHaveBeenDispatched
                | RecoveryOperationState::Accepted
                | RecoveryOperationState::Unknown
        );
        let mut body = json!({
            "result": response_result(
                if unresolved {
                    "UNKNOWN"
                } else {
                    super::recovery_payload::operation_state_name(stored.state)
                },
                unresolved,
                unresolved.then_some(1),
            ),
            "operation": self.recovery_operation_value(&stored),
        });
        if kind == RecoveryKind::OperationLookup {
            body["mutation_authorized"] = Value::Bool(false);
        } else {
            body["witness"] = Value::Null;
        }
        (
            if unresolved { status.max(503) } else { 200 },
            response_frame(kind, correlation, &self.config.caller_id, body),
        )
    }
}

#[derive(Clone, Debug)]
struct HostOperationEvidence {
    result_status: String,
    operation_state: RecoveryOperationState,
    ticket: Option<RecoveryAdmissionTicket>,
    witness: Option<RecoveryEffectWitness>,
}
