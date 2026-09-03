// SPDX-License-Identifier: MIT

#[derive(Clone, Debug)]
struct RuntimeV2Operation {
    request_digest: RuntimeV2RequestDigest,
    canonical_request: Vec<u8>,
    request: RuntimeV2Message,
    result: Option<RuntimeV2Message>,
}

/// Bounded Runtime-v2 operation ledger with optional owner-managed persistence.
pub struct RuntimeV2Ledger<P> {
    config: RuntimeV2LedgerConfig,
    binding: RuntimeV2Binding,
    forwarding: P,
    operations: BTreeMap<RuntimeV2OperationKey, RuntimeV2Operation>,
}

impl<P> RuntimeV2Ledger<P>
where
    P: RuntimeV2ForwardingPort,
{
    /// Creates a ledger with a fixed retained-operation capacity.
    pub fn new(
        config: RuntimeV2LedgerConfig,
        binding: RuntimeV2Binding,
        forwarding: P,
    ) -> Result<Self, RuntimeV2LedgerError> {
        if config.operation_capacity == 0 {
            return Err(RuntimeV2LedgerError::ZeroCapacity);
        }
        Ok(Self {
            config,
            binding,
            forwarding,
            operations: BTreeMap::new(),
        })
    }

    /// Returns the current gateway-held observation.
    pub const fn observation(&self) -> RuntimeV2Observation {
        self.binding.observation
    }

    pub fn binding(&self) -> &RuntimeV2Binding {
        &self.binding
    }

    pub const fn operation_capacity(&self) -> usize {
        self.config.operation_capacity
    }

    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    pub fn forwarding_mut(&mut self) -> &mut P {
        &mut self.forwarding
    }

    /// Accepts one validated read-only state response and advances the binding observation.
    ///
    /// A gateway must refresh its optimistic-concurrency generation from the authoritative mod
    /// before admitting a subsequent action. This method only records the observed state; it
    /// never dispatches or retries mutation-bearing work.
    pub fn accept_state_response(
        &mut self,
        request: &RuntimeV2Message,
        response: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        if request.validate().is_err()
            || request.kind != RuntimeV2MessageKind::StateRequest
            || response.validate().is_err()
            || response.kind != RuntimeV2MessageKind::StateResponse
            || !self.response_matches_request(request, &response)
        {
            return Err(RuntimeV2LedgerError::InvalidRequest(
                RuntimeV2ValidationError::ResultShape,
            ));
        }
        self.validate_context(request)?;
        let Some(observation) = response.observation else {
            return Err(RuntimeV2LedgerError::InvalidRequest(
                RuntimeV2ValidationError::ResultShape,
            ));
        };
        self.binding.observation = observation;
        Ok(response)
    }

    /// Records cancellation only before dispatch; it never cancels admitted work.
    pub fn cancel_before_dispatch(
        &mut self,
        request: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.validate_action_request(&request)?;
        let key = self.key_for(&request)?;
        let canonical_request = request
            .idempotency_canonical_json()
            .map_err(RuntimeV2LedgerError::RequestDigest)?;
        let digest = request
            .request_digest()
            .map_err(RuntimeV2LedgerError::RequestDigest)?;
        self.validate_context(&request)?;
        if let Some(existing) = self.operations.get(&key) {
            return self.replay_or_conflict(existing, &request, digest, &canonical_request);
        }
        if self.operations.len() >= self.config.operation_capacity {
            return Err(RuntimeV2LedgerError::CapacityExceeded);
        }
        if request.generation != self.binding.observation.generation {
            return Err(RuntimeV2LedgerError::StaleGeneration {
                expected: self.binding.observation.generation,
                actual: request.generation,
            });
        }
        let response = self.result_response(
            &request,
            RuntimeV2Status::Cancelled,
            Some(self.binding.observation),
            Some(String::from("sts2.runtime/cancelled_before_dispatch")),
            None,
            RuntimeV2MessageKind::ActionResponse,
        )?;
        self.retain(key, digest, canonical_request, request, response.clone());
        Ok(response)
    }

    /// Reconciles an accepted or unknown operation by reading a retained receipt only.
    pub fn reconcile(
        &mut self,
        request: RuntimeV2Message,
    ) -> Result<RuntimeV2Message, RuntimeV2LedgerError> {
        self.validate_reconcile_request(&request)?;
        let key = self.key_for(&request)?;
        self.validate_context(&request)?;
        if request.generation != self.binding.observation.generation {
            return Err(RuntimeV2LedgerError::StaleGeneration {
                expected: self.binding.observation.generation,
                actual: request.generation,
            });
        }
        let (original_request, original_result) = {
            let Some(operation) = self.operations.get(&key) else {
                return Err(RuntimeV2LedgerError::OperationNotFound);
            };
            let Some(result) = operation.result.clone() else {
                return Err(RuntimeV2LedgerError::OperationInProgress);
            };
            (operation.request.clone(), result)
        };
        if !matches!(
            original_result.status,
            Some(RuntimeV2Status::Accepted | RuntimeV2Status::Unknown)
        ) {
            return Ok(self.as_reconcile_response(&original_result, &request));
        }
        let action = original_request
            .action
            .clone()
            .ok_or(RuntimeV2LedgerError::MissingOperationId)?;
        let receipt_request = RuntimeV2ReceiptRequest::new(request.clone(), key.clone(), action);
        let receipt = self
            .forwarding
            .read_runtime_v2_receipt(receipt_request)
            .unwrap_or_default();
        let Some(receipt) = receipt else {
            return Ok(self.as_reconcile_response(&original_result, &request));
        };
        let Some(receipt_result) =
            self.accept_receipt(&original_request, &request, receipt)
        else {
            return Ok(self.as_reconcile_response(&original_result, &request));
        };
        if let Some(operation) = self.operations.get_mut(&key) {
            operation.result = Some(receipt_result.clone());
        }
        Ok(self.as_reconcile_response(&receipt_result, &request))
    }
}
