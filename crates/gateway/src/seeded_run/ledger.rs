// SPDX-License-Identifier: MIT

use super::*;

#[path = "ledger_persistence.rs"]
mod persistence;
#[path = "ledger_responses.rs"]
mod responses;

pub use persistence::{SeededRunPersistedOperation, SeededRunPersistedState};
use responses::{as_reconcile_response, canonical_request, replay_for_request, safe_identity};

/// Fixed identity and lease fence for one isolated downstream instance.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SeededRunBinding {
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub generation: u64,
}

impl SeededRunBinding {
    pub fn new(
        instance_id: impl Into<String>,
        session_id: impl Into<String>,
        lease_id: impl Into<String>,
        lease_epoch: u64,
        generation: u64,
    ) -> Result<Self, SeededRunLedgerError> {
        let binding = Self {
            instance_id: instance_id.into(),
            session_id: session_id.into(),
            lease_id: lease_id.into(),
            lease_epoch,
            generation,
        };
        for identity in [&binding.instance_id, &binding.session_id, &binding.lease_id] {
            if !safe_identity(identity) {
                return Err(SeededRunLedgerError::InvalidBinding);
            }
        }
        if lease_epoch > SEEDED_RUN_MAX_GENERATION || generation > SEEDED_RUN_MAX_GENERATION {
            return Err(SeededRunLedgerError::InvalidBinding);
        }
        Ok(binding)
    }

    fn matches(&self, request: &SeededRunMessage) -> bool {
        self.instance_id == request.instance_id
            && self.session_id == request.session_id
            && self.lease_id == request.lease_id
            && self.lease_epoch == request.lease_epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeededRunLedgerConfig {
    operation_capacity: usize,
}

impl SeededRunLedgerConfig {
    #[must_use]
    pub const fn new(operation_capacity: usize) -> Self {
        Self { operation_capacity }
    }
}

#[derive(Clone, Debug)]
struct SeededRunOperation {
    request_digest: Vec<u8>,
    request: SeededRunMessage,
    result: Option<SeededRunMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeededRunForwardRequest {
    pub message: SeededRunMessage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeededRunReceiptRequest {
    pub message: SeededRunMessage,
    pub operation_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeededRunTransportFault {
    UnavailableBeforeWrite,
    Timeout,
    MalformedResponse,
}

pub trait SeededRunForwardingPort {
    fn forward_seeded_run(
        &mut self,
        request: SeededRunForwardRequest,
    ) -> Result<SeededRunMessage, SeededRunTransportFault>;

    fn read_seeded_run_receipt(
        &mut self,
        request: SeededRunReceiptRequest,
    ) -> Result<Option<SeededRunMessage>, SeededRunTransportFault>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeededRunLedgerError {
    ZeroCapacity,
    InvalidBinding,
    InvalidRequest,
    InvalidResponse,
    RequestDigest,
    CapacityExceeded,
    OperationNotFound,
    OperationInProgress,
    OperationConflict,
    StaleGeneration,
    FenceRejected,
    PersistenceFailed,
}

/// Bounded, idempotent, lease-fenced ledger. It forwards a start at most once and only
/// performs read-only receipt lookup during reconciliation.
pub struct SeededRunLedger<P> {
    config: SeededRunLedgerConfig,
    binding: SeededRunBinding,
    forwarding: P,
    operations: BTreeMap<String, SeededRunOperation>,
}

impl<P: SeededRunForwardingPort> SeededRunLedger<P> {
    pub fn new(
        config: SeededRunLedgerConfig,
        binding: SeededRunBinding,
        forwarding: P,
    ) -> Result<Self, SeededRunLedgerError> {
        if config.operation_capacity == 0 {
            return Err(SeededRunLedgerError::ZeroCapacity);
        }
        Ok(Self {
            config,
            binding,
            forwarding,
            operations: BTreeMap::new(),
        })
    }

    pub fn binding(&self) -> &SeededRunBinding {
        &self.binding
    }

    pub fn forwarding_mut(&mut self) -> &mut P {
        &mut self.forwarding
    }

    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    pub fn submit_start(
        &mut self,
        request: SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        self.submit_start_with_checkpoint(request, |_| Ok(()))
    }

    pub fn submit_start_with_checkpoint<F>(
        &mut self,
        request: SeededRunMessage,
        mut checkpoint: F,
    ) -> Result<SeededRunMessage, SeededRunLedgerError>
    where
        F: FnMut(&Self) -> Result<(), ()>,
    {
        request.validate()?;
        if request.kind != SeededRunMessageKind::StartRequest {
            return Err(SeededRunLedgerError::InvalidRequest);
        }
        self.validate_binding(&request)?;
        let digest = canonical_request(&request)?;
        if let Some(existing) = self.operations.get(&request.operation_id) {
            if existing.request_digest != digest {
                return Err(SeededRunLedgerError::OperationConflict);
            }
            return existing
                .result
                .clone()
                .map(|result| replay_for_request(result, &request))
                .ok_or(SeededRunLedgerError::OperationInProgress);
        }
        if self.operations.len() >= self.config.operation_capacity {
            return Err(SeededRunLedgerError::CapacityExceeded);
        }
        // A newly attached gateway has no authoritative observation of the native host yet.
        // The caller's fenced generation is used only to initialize this empty binding; the
        // host remains authoritative because its response must carry a matching lineage and a
        // fresh settlement generation. Once any operation is retained, the fence is exact.
        if self.operations.is_empty() && request.generation >= self.binding.generation {
            self.binding.generation = request.generation;
        }
        if request.generation != self.binding.generation {
            let response = Self::rejected(&request, "stale_generation")?;
            self.retain(request, digest, response.clone());
            checkpoint(self).map_err(|_| SeededRunLedgerError::PersistenceFailed)?;
            return Ok(response);
        }
        let operation_id = request.operation_id.clone();
        self.operations.insert(
            operation_id.clone(),
            SeededRunOperation {
                request: request.clone(),
                request_digest: digest,
                result: None,
            },
        );
        if checkpoint(self).is_err() {
            self.operations.remove(&operation_id);
            return Err(SeededRunLedgerError::PersistenceFailed);
        }
        let result = match self.forwarding.forward_seeded_run(SeededRunForwardRequest {
            message: request.clone(),
        }) {
            Ok(response) => self.accept_forwarded(&request, response),
            Err(SeededRunTransportFault::Timeout) => {
                Ok(Self::unknown(&request, "downstream_timeout")?)
            }
            Err(SeededRunTransportFault::UnavailableBeforeWrite) => {
                Ok(Self::unknown(&request, "downstream_unavailable")?)
            }
            Err(SeededRunTransportFault::MalformedResponse) => {
                Ok(Self::unknown(&request, "downstream_malformed_response")?)
            }
        }?;
        let operation = self
            .operations
            .get_mut(&operation_id)
            .ok_or(SeededRunLedgerError::OperationNotFound)?;
        operation.result = Some(result.clone());
        self.advance_generation(&result);
        checkpoint(self).map_err(|_| SeededRunLedgerError::PersistenceFailed)?;
        Ok(result)
    }

    pub fn reconcile(
        &mut self,
        request: SeededRunMessage,
    ) -> Result<SeededRunMessage, SeededRunLedgerError> {
        request.validate()?;
        if request.kind != SeededRunMessageKind::ReconcileRequest {
            return Err(SeededRunLedgerError::InvalidRequest);
        }
        self.validate_binding(&request)?;
        if request.generation != self.binding.generation {
            return Err(SeededRunLedgerError::StaleGeneration);
        }
        let operation = self
            .operations
            .get(&request.operation_id)
            .ok_or(SeededRunLedgerError::OperationNotFound)?;
        let result = operation
            .result
            .clone()
            .ok_or(SeededRunLedgerError::OperationInProgress)?;
        if !matches!(
            result.status,
            Some(SeededRunStatus::Accepted | SeededRunStatus::Unknown)
        ) {
            return Ok(as_reconcile_response(result, &request));
        }
        let original = operation.request.clone();
        let receipt = self
            .forwarding
            .read_seeded_run_receipt(SeededRunReceiptRequest {
                message: request.clone(),
                operation_id: request.operation_id.clone(),
            })
            .unwrap_or_default();
        let Some(receipt) = receipt else {
            return Ok(as_reconcile_response(result, &request));
        };
        let receipt = match self.accept_receipt(&original, &request, receipt) {
            Ok(receipt) => receipt,
            Err(_) => return Ok(as_reconcile_response(result, &request)),
        };
        let mut stored = receipt.clone();
        stored.kind = SeededRunMessageKind::StartResponse;
        stored.correlation_id = original.correlation_id.clone();
        if let Some(operation) = self.operations.get_mut(&request.operation_id) {
            operation.result = Some(stored.clone());
        }
        self.advance_generation(&stored);
        Ok(as_reconcile_response(receipt, &request))
    }
}
