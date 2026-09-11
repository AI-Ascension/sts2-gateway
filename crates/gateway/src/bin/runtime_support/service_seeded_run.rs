// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn seeded_run_start_path(&self) -> String {
        format!("/v2/instances/{}/seeded-run", self.config.instance_id)
    }

    pub(super) fn seeded_run_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!(
            "/v2/instances/{}/seeded-operations/",
            self.config.instance_id
        );
        path.strip_prefix(&prefix)
            .filter(|operation_id| safe_operation_id(operation_id))
    }

    pub(super) fn seeded_run_start(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if request.body.is_empty() || request.body.len() > MAX_BODY_BYTES {
            return (413, json_error("seeded_run_body_oversized"));
        }
        let Ok(message) = serde_json::from_slice::<sts2_gateway::SeededRunMessage>(&request.body)
        else {
            return (400, json_error("seeded_run_request_invalid"));
        };
        if request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(message.correlation_id.as_str())
        {
            return (409, json_error("seeded_run_correlation_mismatch"));
        }
        if message.instance_id != self.config.instance_id
            || message.session_id != self.config.session_id
            || message.lease_id != self.config.lease_id
            || message.lease_epoch != self.config.lease_epoch
        {
            return (409, json_error("seeded_run_identity_rejected"));
        }
        let result = match self.journal_path.as_deref() {
            Some(path) => self
                .seeded_run
                .submit_start_with_checkpoint(message, |ledger| {
                    journal::seeded_store(path, &ledger.persisted_state()).map_err(|_| ())
                }),
            None => self.seeded_run.submit_start(message),
        };
        match result {
            Ok(response) => (200, seeded_run_bytes(&response)),
            Err(error) => seeded_run_error(error),
        }
    }

    pub(super) fn seeded_run_reconcile(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let message = sts2_gateway::SeededRunMessage::reconcile_request(
            sts2_gateway::SeededRunProvenance::default(),
            sts2_gateway::SeededRunContext::new(
                correlation_id.clone(),
                self.config.instance_id.clone(),
                self.config.session_id.clone(),
                self.config.lease_id.clone(),
                self.config.lease_epoch,
                self.seeded_run.binding().generation,
            ),
            operation_id.to_owned(),
        );
        match self.seeded_run.reconcile(message) {
            Ok(response) => {
                if let Some(path) = self.journal_path.as_deref()
                    && journal::seeded_store(path, &self.seeded_run.persisted_state()).is_err()
                {
                    return (503, json_error("seeded_run_persistence_uncertain"));
                }
                (200, seeded_run_bytes(&response))
            }
            Err(error) => seeded_run_error(error),
        }
    }
}

fn seeded_run_bytes(message: &sts2_gateway::SeededRunMessage) -> Vec<u8> {
    serde_json::to_vec(message).unwrap_or_else(|_| json_error("seeded_run_serialization_failed"))
}

fn seeded_run_error(error: sts2_gateway::SeededRunLedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        sts2_gateway::SeededRunLedgerError::InvalidRequest
        | sts2_gateway::SeededRunLedgerError::InvalidResponse
        | sts2_gateway::SeededRunLedgerError::RequestDigest => (400, "seeded_run_request_invalid"),
        sts2_gateway::SeededRunLedgerError::CapacityExceeded => {
            return (429, json_overload("seeded_run_operation_capacity"));
        }
        sts2_gateway::SeededRunLedgerError::OperationNotFound => {
            (404, "seeded_run_operation_not_found")
        }
        sts2_gateway::SeededRunLedgerError::OperationInProgress => {
            (409, "seeded_run_operation_in_progress")
        }
        sts2_gateway::SeededRunLedgerError::OperationConflict => {
            (409, "seeded_run_idempotency_conflict")
        }
        sts2_gateway::SeededRunLedgerError::StaleGeneration => (409, "seeded_run_stale_generation"),
        sts2_gateway::SeededRunLedgerError::FenceRejected => {
            (409, "seeded_run_lease_fence_rejected")
        }
        sts2_gateway::SeededRunLedgerError::ZeroCapacity
        | sts2_gateway::SeededRunLedgerError::InvalidBinding
        | sts2_gateway::SeededRunLedgerError::PersistenceFailed => {
            (500, "seeded_run_configuration_invalid")
        }
    };
    (status, json_error(code))
}
