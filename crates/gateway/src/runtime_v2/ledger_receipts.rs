// SPDX-License-Identifier: MIT

impl<P> RuntimeV2Ledger<P>
where
    P: RuntimeV2ForwardingPort,
{
    fn response_for_transport_fault(
        &self,
        request: &RuntimeV2Message,
        fault: RuntimeV2TransportFault,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        match fault {
            RuntimeV2TransportFault::UnavailableBeforeWrite
            | RuntimeV2TransportFault::RejectedBeforeWrite
            | RuntimeV2TransportFault::TimeoutBeforeWrite
            | RuntimeV2TransportFault::DisconnectedBeforeWrite => self.rejected_response(
                request,
                match fault {
                    RuntimeV2TransportFault::UnavailableBeforeWrite => {
                        "sts2.runtime/downstream_unavailable"
                    }
                    RuntimeV2TransportFault::RejectedBeforeWrite => {
                        "sts2.runtime/downstream_rejected"
                    }
                    RuntimeV2TransportFault::TimeoutBeforeWrite => {
                        "sts2.runtime/timeout_before_write"
                    }
                    RuntimeV2TransportFault::DisconnectedBeforeWrite => {
                        "sts2.runtime/disconnected_before_write"
                    }
                    _ => "sts2.runtime/dispatch_rejected",
                },
                self.binding.observation,
            ),
            RuntimeV2TransportFault::TimeoutAfterWrite
            | RuntimeV2TransportFault::DisconnectedAfterWrite
            | RuntimeV2TransportFault::MalformedResponse
            | RuntimeV2TransportFault::ReceiptUnavailable => self.result_response(
                request,
                RuntimeV2Status::Unknown,
                None,
                Some(String::from("sts2.runtime/unknown_after_disconnect")),
                None,
                RuntimeV2MessageKind::ActionResponse,
            ),
        }
    }

    fn accept_forwarded_response(
        &mut self,
        request: &RuntimeV2Message,
        response: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        if !self.response_matches_request(request, &response)
            || response.validate().is_err()
            || response.kind != RuntimeV2MessageKind::ActionResponse
        {
            return self.result_response(
                request,
                RuntimeV2Status::Unknown,
                None,
                Some(String::from("sts2.runtime/unknown_after_disconnect")),
                None,
                RuntimeV2MessageKind::ActionResponse,
            );
        }
        let Some(status) = response.status else {
            return self.result_response(
                request,
                RuntimeV2Status::Unknown,
                None,
                Some(String::from("sts2.runtime/unknown_after_disconnect")),
                None,
                RuntimeV2MessageKind::ActionResponse,
            );
        };
        match status {
            RuntimeV2Status::Accepted => {
                if response.observation.is_some_and(|observation| {
                    observation.generation == request.generation
                        && observation == self.binding.observation
                }) && response.effect_witness.is_none()
                    && response.error_code.is_none()
                {
                    Ok(response)
                } else {
                    self.result_response(
                        request,
                        RuntimeV2Status::Unknown,
                        None,
                        Some(String::from("sts2.runtime/unknown_after_disconnect")),
                        None,
                        RuntimeV2MessageKind::ActionResponse,
                    )
                }
            }
            RuntimeV2Status::Settled => {
                let fresh = response
                    .observation
                    .is_some_and(|observation| observation.generation > request.generation);
                if fresh
                    && response
                        .effect_witness
                        .as_ref()
                        .is_some_and(|witness| witness.generation == response.generation)
                {
                    if let Some(observation) = response.observation {
                        self.binding.observation = observation;
                    }
                    Ok(response)
                } else {
                    self.result_response(
                        request,
                        RuntimeV2Status::Unknown,
                        None,
                        Some(String::from("sts2.runtime/unknown_after_disconnect")),
                        None,
                        RuntimeV2MessageKind::ActionResponse,
                    )
                }
            }
            RuntimeV2Status::Rejected => {
                if response.observation.is_some_and(|observation| {
                    observation.generation == self.binding.observation.generation
                }) && response.effect_witness.is_none()
                    && response.error_code.is_some()
                {
                    Ok(response)
                } else {
                    self.result_response(
                        request,
                        RuntimeV2Status::Unknown,
                        None,
                        Some(String::from("sts2.runtime/unknown_after_disconnect")),
                        None,
                        RuntimeV2MessageKind::ActionResponse,
                    )
                }
            }
            RuntimeV2Status::Unknown | RuntimeV2Status::Cancelled => self.result_response(
                request,
                RuntimeV2Status::Unknown,
                None,
                Some(String::from("sts2.runtime/unknown_after_disconnect")),
                None,
                RuntimeV2MessageKind::ActionResponse,
            ),
        }
    }

    fn accept_receipt(
        &mut self,
        action_request: &RuntimeV2Message,
        reconcile_request: &RuntimeV2Message,
        mut receipt: RuntimeV2Message,
    ) -> Option<RuntimeV2Message> {
        if receipt.protocol_version != action_request.protocol_version
            || receipt.schema_digest != action_request.schema_digest
            || receipt.provenance != action_request.provenance
            || receipt.correlation_id != reconcile_request.correlation_id
            || receipt.instance_id != action_request.instance_id
            || receipt.session_id != action_request.session_id
            || receipt.lease_id != action_request.lease_id
            || receipt.lease_epoch != action_request.lease_epoch
            || receipt.operation_id != action_request.operation_id
            || receipt.action != action_request.action
            || receipt.validate().is_err()
            || receipt.kind != RuntimeV2MessageKind::ActionResponse
        {
            return None;
        }
        // The stored result must remain replayable for the original action request. The
        // reconciliation response gets its caller-specific correlation below in
        // `as_reconcile_response`.
        receipt.correlation_id = action_request.correlation_id.clone();
        match receipt.status {
            Some(RuntimeV2Status::Settled)
                if receipt
                    .observation
                    .is_some_and(|observation| observation.generation > action_request.generation)
                    && receipt
                        .effect_witness
                        .as_ref()
                        .is_some_and(|witness| witness.generation == receipt.generation) =>
            {
                if let Some(observation) = receipt.observation.filter(|observation| {
                    observation.generation > self.binding.observation.generation
                }) {
                    self.binding.observation = observation;
                }
                Some(receipt)
            }
            Some(RuntimeV2Status::Accepted | RuntimeV2Status::Rejected)
                if receipt.observation.is_some_and(|observation| {
                    observation.generation == action_request.generation
                }) =>
            {
                Some(receipt)
            }
            _ => None,
        }
    }

    fn response_matches_request(
        &self,
        request: &RuntimeV2Message,
        response: &RuntimeV2Message,
    ) -> bool {
        response.protocol_version == request.protocol_version
            && response.schema_digest == request.schema_digest
            && response.provenance == request.provenance
            && response.correlation_id == request.correlation_id
            && response.instance_id == request.instance_id
            && response.session_id == request.session_id
            && response.lease_id == request.lease_id
            && response.lease_epoch == request.lease_epoch
            && response.operation_id == request.operation_id
            && response.action == request.action
    }

    fn as_reconcile_response(
        &self,
        result: &RuntimeV2Message,
        request: &RuntimeV2Message,
    ) -> RuntimeV2Message {
        let mut response = result.clone();
        response.kind = RuntimeV2MessageKind::ReconcileResponse;
        response.correlation_id = request.correlation_id.clone();
        response
    }
}
