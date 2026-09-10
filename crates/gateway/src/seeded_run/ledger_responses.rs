// SPDX-License-Identifier: MIT

use super::*;

impl From<SeededRunValidationError> for SeededRunLedgerError {
    fn from(_: SeededRunValidationError) -> Self {
        Self::InvalidRequest
    }
}

impl fmt::Display for SeededRunLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroCapacity => "seeded-run operation capacity must be positive",
            Self::InvalidBinding => "seeded-run gateway binding is invalid",
            Self::InvalidRequest => "seeded-run request is invalid",
            Self::InvalidResponse => "seeded-run response is invalid",
            Self::RequestDigest => "seeded-run request digest failed",
            Self::CapacityExceeded => "seeded-run operation store is full",
            Self::OperationNotFound => "seeded-run operation was not retained",
            Self::OperationInProgress => "seeded-run operation is still being dispatched",
            Self::OperationConflict => "seeded-run operation was reused with different semantics",
            Self::StaleGeneration => "seeded-run request generation is stale",
            Self::FenceRejected => "seeded-run request was rejected by the lease fence",
            Self::PersistenceFailed => "seeded-run durable checkpoint failed",
        })
    }
}

impl std::error::Error for SeededRunLedgerError {}

impl<P: SeededRunForwardingPort> SeededRunLedger<P> {
    pub(super) fn validate_binding(
        &self,
        request: &SeededRunMessage,
    ) -> Result<(), SeededRunLedgerError> {
        if self.binding.matches(request) {
            Ok(())
        } else {
            Err(SeededRunLedgerError::FenceRejected)
        }
    }

    pub(super) fn accept_forwarded(
        &self,
        request: &SeededRunMessage,
        response: SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        response
            .validate()
            .map_err(|_| SeededRunLedgerError::InvalidResponse)?;
        if !matches!(
            response.kind,
            SeededRunMessageKind::StartResponse | SeededRunMessageKind::ReconcileResponse
        ) || response.correlation_id != request.correlation_id
            || response.instance_id != request.instance_id
            || response.session_id != request.session_id
            || response.lease_id != request.lease_id
            || response.lease_epoch != request.lease_epoch
            || response.generation != request.generation
            || response.operation_id != request.operation_id
            || response.requested_seed != request.requested_seed
            || response.run_mode != request.run_mode
            || response.context_digest != request.context_digest
            || response.selected_context != request.selected_context
        {
            return Err(SeededRunLedgerError::InvalidResponse);
        }
        Ok(response)
    }

    pub(super) fn accept_receipt(
        &self,
        original: &SeededRunMessage,
        reconcile: &SeededRunMessage,
        response: SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        response
            .validate()
            .map_err(|_| SeededRunLedgerError::InvalidResponse)?;
        if response.kind != SeededRunMessageKind::ReconcileResponse
            || response.correlation_id != reconcile.correlation_id
            || response.instance_id != original.instance_id
            || response.session_id != original.session_id
            || response.lease_id != original.lease_id
            || response.lease_epoch != original.lease_epoch
            || response.generation != original.generation
            || response.operation_id != original.operation_id
            || response.requested_seed != original.requested_seed
            || response.run_mode != original.run_mode
            || response.context_digest != original.context_digest
            || response.selected_context != original.selected_context
        {
            return Err(SeededRunLedgerError::InvalidResponse);
        }
        Ok(response)
    }

    pub(super) fn advance_generation(&mut self, response: &SeededRunMessage) {
        if response.status == Some(SeededRunStatus::Settled)
            && let Some(observation) = &response.observation
            && observation.generation > self.binding.generation
        {
            self.binding.generation = observation.generation;
        }
    }

    pub(super) fn retain(
        &mut self,
        request: SeededRunMessage,
        digest: Vec<u8>,
        response: SeededRunMessage,
    ) {
        self.operations.insert(
            request.operation_id.clone(),
            SeededRunOperation {
                request,
                request_digest: digest,
                result: Some(response),
            },
        );
    }

    pub(super) fn rejected(
        request: &SeededRunMessage,
        error_code: &str,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        let context = request
            .selected_context
            .clone()
            .ok_or(SeededRunLedgerError::InvalidRequest)?;
        Ok(SeededRunMessage::result_with_context(
            request.provenance.clone(),
            SeededRunContext::new(
                request.correlation_id.clone(),
                request.instance_id.clone(),
                request.session_id.clone(),
                request.lease_id.clone(),
                request.lease_epoch,
                request.generation,
            ),
            SeededRunMessageKind::StartResponse,
            request.operation_id.clone(),
            request
                .requested_seed
                .clone()
                .ok_or(SeededRunLedgerError::InvalidRequest)?,
            request
                .run_mode
                .ok_or(SeededRunLedgerError::InvalidRequest)?,
            context,
            SeededRunStatus::Rejected,
            None,
            None,
            None,
            Some(error_code.to_owned()),
        ))
    }

    pub(super) fn unknown(
        request: &SeededRunMessage,
        error_code: &str,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        let context = request
            .selected_context
            .clone()
            .ok_or(SeededRunLedgerError::InvalidRequest)?;
        Ok(SeededRunMessage::result_with_context(
            request.provenance.clone(),
            SeededRunContext::new(
                request.correlation_id.clone(),
                request.instance_id.clone(),
                request.session_id.clone(),
                request.lease_id.clone(),
                request.lease_epoch,
                request.generation,
            ),
            SeededRunMessageKind::StartResponse,
            request.operation_id.clone(),
            request
                .requested_seed
                .clone()
                .ok_or(SeededRunLedgerError::InvalidRequest)?,
            request
                .run_mode
                .ok_or(SeededRunLedgerError::InvalidRequest)?,
            context,
            SeededRunStatus::Unknown,
            None,
            None,
            None,
            Some(error_code.to_owned()),
        ))
    }
}

pub(super) fn canonical_request(
    request: &SeededRunMessage,
) -> Result<Vec<u8>, SeededRunLedgerError> {
    // Correlation identifies this transport attempt (and is derived from the JSON-RPC id),
    // rather than the semantic operation. A retried start with a fresh RPC id must therefore
    // hit the retained operation and replay its result without forwarding a second native start.
    let mut identity = request.clone();
    identity.correlation_id.clear();
    serde_json::to_vec(&identity).map_err(|_| SeededRunLedgerError::RequestDigest)
}

pub(super) fn replay_for_request(
    mut result: SeededRunMessage,
    request: &SeededRunMessage,
) -> SeededRunMessage {
    result.correlation_id = request.correlation_id.clone();
    result
}

pub(super) fn as_reconcile_response(
    mut response: SeededRunMessage,
    request: &SeededRunMessage,
) -> SeededRunMessage {
    response.kind = SeededRunMessageKind::ReconcileResponse;
    response.correlation_id = request.correlation_id.clone();
    response
}

pub(super) fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= SEEDED_RUN_MAX_IDENTITY_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
