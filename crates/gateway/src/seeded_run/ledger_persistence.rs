// SPDX-License-Identifier: MIT

use super::*;

/// Durable gateway-owned marker used to recover an accepted operation without replaying it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeededRunPersistedOperation {
    pub request: SeededRunMessage,
    pub result: Option<SeededRunMessage>,
}

/// Durable seeded-run ledger state. A missing result is deliberately recovered as `unknown`.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeededRunPersistedState {
    pub binding: SeededRunBinding,
    pub operations: Vec<SeededRunPersistedOperation>,
}

impl<P: SeededRunForwardingPort> SeededRunLedger<P> {
    #[must_use]
    pub fn persisted_state(&self) -> SeededRunPersistedState {
        SeededRunPersistedState {
            binding: self.binding.clone(),
            operations: self
                .operations
                .values()
                .map(|operation| SeededRunPersistedOperation {
                    request: operation.request.clone(),
                    result: operation.result.clone(),
                })
                .collect(),
        }
    }

    /// Restores markers from disk and converts accepted/in-flight work to explicit unknown.
    pub fn restore_state(
        &mut self,
        persisted: SeededRunPersistedState,
    ) -> Result<(), SeededRunLedgerError> {
        let validated_binding = SeededRunBinding::new(
            persisted.binding.instance_id.clone(),
            persisted.binding.session_id.clone(),
            persisted.binding.lease_id.clone(),
            persisted.binding.lease_epoch,
            persisted.binding.generation,
        )
        .map_err(|_| SeededRunLedgerError::InvalidBinding)?;
        if persisted.binding.instance_id != self.binding.instance_id
            || persisted.binding.session_id != self.binding.session_id
            || persisted.binding.lease_id != self.binding.lease_id
            || persisted.binding.lease_epoch != self.binding.lease_epoch
            || validated_binding.generation < self.binding.generation
            || persisted.operations.len() > self.config.operation_capacity
        {
            return Err(SeededRunLedgerError::InvalidBinding);
        }
        let mut operations = BTreeMap::new();
        for item in persisted.operations {
            item.request.validate()?;
            if item.request.kind != SeededRunMessageKind::StartRequest
                || !self.binding.matches(&item.request)
                || operations.contains_key(&item.request.operation_id)
            {
                return Err(SeededRunLedgerError::InvalidRequest);
            }
            let digest = canonical_request(&item.request)?;
            let result = match item.result {
                Some(result) => {
                    self.accept_forwarded(&item.request, result.clone())?;
                    if result.kind != SeededRunMessageKind::StartResponse
                        || result.observation.as_ref().is_some_and(|observation| {
                            observation.generation > persisted.binding.generation
                        })
                    {
                        return Err(SeededRunLedgerError::InvalidResponse);
                    }
                    if result.status == Some(SeededRunStatus::Accepted) {
                        Self::unknown(&item.request, "restart_uncertain")?
                    } else {
                        result
                    }
                }
                None => Self::unknown(&item.request, "restart_uncertain")?,
            };
            operations.insert(
                item.request.operation_id.clone(),
                SeededRunOperation {
                    request: item.request,
                    request_digest: digest,
                    result: Some(result),
                },
            );
        }
        self.binding.generation = persisted.binding.generation;
        self.operations = operations;
        Ok(())
    }
}
