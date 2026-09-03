// SPDX-License-Identifier: MIT

impl<P> RuntimeV2Ledger<P>
where
    P: RuntimeV2ForwardingPort,
{
    fn validate_action_request(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<(), RuntimeV2LedgerError> {
        request
            .validate()
            .map_err(RuntimeV2LedgerError::InvalidRequest)?;
        if request.kind != RuntimeV2MessageKind::ActionRequest {
            return Err(RuntimeV2LedgerError::InvalidRequest(
                RuntimeV2ValidationError::ResultShape,
            ));
        }
        Ok(())
    }

    fn validate_reconcile_request(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<(), RuntimeV2LedgerError> {
        request
            .validate()
            .map_err(RuntimeV2LedgerError::InvalidRequest)?;
        if request.kind != RuntimeV2MessageKind::ReconcileRequest {
            return Err(RuntimeV2LedgerError::InvalidRequest(
                RuntimeV2ValidationError::ResultShape,
            ));
        }
        Ok(())
    }

    fn key_for(
        &self,
        request: &RuntimeV2Message,
    ) -> Result<RuntimeV2OperationKey, RuntimeV2LedgerError> {
        request
            .operation_key()
            .ok_or(RuntimeV2LedgerError::MissingOperationId)
    }

    fn validate_context(&self, request: &RuntimeV2Message) -> Result<(), RuntimeV2LedgerError> {
        if let Some(failure) = self.binding.fence_failure(request) {
            return Err(RuntimeV2LedgerError::Fence(failure));
        }
        Ok(())
    }

    fn replay_or_conflict(
        &self,
        existing: &RuntimeV2Operation,
        request: &RuntimeV2Message,
        digest: RuntimeV2RequestDigest,
        canonical_request: &[u8],
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        if existing.request_digest == digest && existing.canonical_request == canonical_request {
            let mut replay = existing
                .result
                .clone()
                .ok_or(RuntimeV2LedgerError::OperationInProgress)?;
            replay.correlation_id = request.correlation_id.clone();
            Ok(replay)
        } else {
            self.result_response(
                request,
                RuntimeV2Status::Rejected,
                Some(self.binding.observation),
                Some(String::from("idempotency_conflict")),
                None,
                RuntimeV2MessageKind::ActionResponse,
            )
        }
    }

    fn retain(
        &mut self,
        key: RuntimeV2OperationKey,
        request_digest: RuntimeV2RequestDigest,
        canonical_request: Vec<u8>,
        request: RuntimeV2Message,
        result: RuntimeV2Message,
    ) {
        self.operations.insert(
            key,
            RuntimeV2Operation {
                request_digest,
                canonical_request,
                request,
                result: Some(result),
            },
        );
    }

    fn rejected_response(
        &self,
        request: &RuntimeV2Message,
        error_code: &str,
        observation: RuntimeV2Observation,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.result_response(
            request,
            RuntimeV2Status::Rejected,
            Some(observation),
            Some(error_code.to_owned()),
            None,
            RuntimeV2MessageKind::ActionResponse,
        )
    }

    fn result_response(
        &self,
        request: &RuntimeV2Message,
        status: RuntimeV2Status,
        observation: Option<RuntimeV2Observation>,
        error_code: Option<String>,
        effect_witness: Option<RuntimeV2EffectWitness>,
        kind: RuntimeV2MessageKind,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        let Some(operation_id) = request.operation_id.as_deref() else {
            return Err(RuntimeV2LedgerError::MissingOperationId);
        };
        let Some(action) = request.action.clone() else {
            return Err(RuntimeV2LedgerError::InvalidRequest(
                RuntimeV2ValidationError::ResultShape,
            ));
        };
        let generation = observation.map_or(request.generation, |value| value.generation);
        let response = RuntimeV2Message::result(
            self.binding.metadata.clone(),
            &request.correlation_id,
            &self.binding.instance_id,
            &self.binding.session_id,
            &self.binding.lease_id,
            self.binding.lease_epoch,
            generation,
            operation_id,
            action,
            status,
            observation,
            error_code,
            effect_witness,
            kind,
        );
        response
            .validate()
            .map_err(RuntimeV2LedgerError::InvalidRequest)?;
        Ok(response)
    }
}
