// SPDX-License-Identifier: MIT

use super::ledger_response::validate_response;
use super::ledger_types::{
    SaveProfileForwardRequest, SaveProfileForwardingPort, SaveProfileLedgerError,
    SaveProfileOperationRecord, SaveProfileRecordStore, SaveProfileResult, SaveProfileStatus,
    SaveProfileTransportFault,
};
use super::ledger_validation::{
    accepted_result, key, rejected_result, same_operation_request, selected_id, unknown_result,
    validate_request,
};
use super::route::SaveProfileRoute;
use super::types::{
    SaveProfileAuthority, SaveProfileContext, SaveProfileFenceError, SaveProfileId,
};

/// A bounded, fenced, idempotent ledger for save-profile reads and selection.
pub struct SaveProfileLedger<F, S> {
    capacity: usize,
    authority: SaveProfileAuthority,
    forwarding: F,
    store: S,
    records: std::collections::BTreeMap<(String, String), SaveProfileOperationRecord>,
    selected: Option<SaveProfileId>,
    pending_selection: Option<String>,
}

impl<F: SaveProfileForwardingPort, S: SaveProfileRecordStore> SaveProfileLedger<F, S> {
    pub fn new(
        capacity: usize,
        authority: SaveProfileAuthority,
        forwarding: F,
        mut store: S,
    ) -> Result<Self, SaveProfileLedgerError> {
        if capacity == 0 {
            return Err(SaveProfileLedgerError::CapacityExceeded);
        }
        let persisted = store.list()?;
        let selected = persisted
            .iter()
            .filter_map(|record| record.result.as_ref())
            .find(|result| {
                result.route == SaveProfileRoute::Select
                    && result.status == SaveProfileStatus::Settled
            })
            .and_then(|result| result.profile_id.clone());
        let pending_selection = persisted
            .iter()
            .find(|record| {
                record.request.route == SaveProfileRoute::Select
                    && matches!(
                        record.status,
                        SaveProfileStatus::Accepted | SaveProfileStatus::Unknown
                    )
            })
            .map(|record| record.operation_id.clone());
        let records = persisted
            .into_iter()
            .map(|record| {
                (
                    (
                        record.request.context.instance_id.clone(),
                        record.operation_id.clone(),
                    ),
                    record,
                )
            })
            .collect();
        Ok(Self {
            capacity,
            authority,
            forwarding,
            store,
            records,
            selected,
            pending_selection,
        })
    }

    pub fn submit(
        &mut self,
        request: SaveProfileForwardRequest,
        now_millis: u64,
        active_run: bool,
    ) -> Result<SaveProfileResult, SaveProfileLedgerError> {
        self.authority
            .authorize(&request.context, now_millis)
            .map_err(SaveProfileLedgerError::Fence)?;
        validate_request(&request)?;
        if active_run && request.route.is_mutation() {
            return Err(SaveProfileLedgerError::ActiveRun);
        }
        if let Some(existing) = self.records.get(&key(&request)) {
            if !same_operation_request(&existing.request, &request) {
                return Err(SaveProfileLedgerError::OperationConflict);
            }
            return existing
                .result
                .clone()
                .ok_or(SaveProfileLedgerError::OperationInProgress);
        }
        if self.records.len() >= self.capacity {
            return Err(SaveProfileLedgerError::CapacityExceeded);
        }
        if request.route == SaveProfileRoute::Select {
            let profile_id = selected_id(&request)?;
            if self.pending_selection.is_some() || self.selected.as_ref() == Some(profile_id) {
                return Err(SaveProfileLedgerError::DuplicateSelection);
            }
        }
        let record = SaveProfileOperationRecord {
            operation_id: request.operation_id.clone(),
            request: request.clone(),
            status: SaveProfileStatus::Accepted,
            result: None,
        };
        self.store.insert(record.clone())?;
        self.records.insert(key(&request), record);
        if request.route == SaveProfileRoute::Select {
            self.pending_selection = Some(request.operation_id.clone());
        }
        self.dispatch(request)
    }

    pub fn reconcile(
        &mut self,
        context: SaveProfileContext,
        operation_id: &str,
        now_millis: u64,
    ) -> Result<SaveProfileResult, SaveProfileLedgerError> {
        self.authority
            .authorize(&context, now_millis)
            .map_err(SaveProfileLedgerError::Fence)?;
        let key = (context.instance_id.clone(), operation_id.to_owned());
        let Some(record) = self.records.get(&key).cloned() else {
            return Err(SaveProfileLedgerError::OperationNotFound);
        };
        if !record.request.context.same_fence(&context) {
            return Err(SaveProfileLedgerError::Fence(
                SaveProfileFenceError::WrongSession,
            ));
        }
        if let Some(previous) = record.result.as_ref()
            && !matches!(
                previous.status,
                SaveProfileStatus::Accepted | SaveProfileStatus::Unknown
            )
        {
            return Ok(previous.clone());
        }
        let receipt = match self.forwarding.lookup(record.request.clone()) {
            Ok(value) => value,
            Err(error) => return Err(SaveProfileLedgerError::Transport(error)),
        };
        let Some(receipt) = receipt else {
            return Ok(record
                .result
                .unwrap_or_else(|| accepted_result(&record.request)));
        };
        let result = match validate_response(&record.request, receipt) {
            Ok(result) => result,
            Err(error) => {
                let unknown = unknown_result(&record.request);
                self.persist_result(key, unknown)?;
                return Err(error);
            }
        };
        self.persist_result(key, result.clone())?;
        Ok(result)
    }

    pub fn caller_disconnected(
        &mut self,
        context: SaveProfileContext,
        operation_id: &str,
        now_millis: u64,
    ) -> Result<SaveProfileResult, SaveProfileLedgerError> {
        self.authority
            .authorize(&context, now_millis)
            .map_err(SaveProfileLedgerError::Fence)?;
        let key = (context.instance_id.clone(), operation_id.to_owned());
        let Some(record) = self.records.get(&key).cloned() else {
            return Err(SaveProfileLedgerError::OperationNotFound);
        };
        if let Some(result) = record.result.as_ref()
            && !matches!(
                result.status,
                SaveProfileStatus::Accepted | SaveProfileStatus::Unknown
            )
        {
            return Ok(result.clone());
        }
        let unknown = unknown_result(&record.request);
        self.persist_result(key, unknown.clone())?;
        Ok(unknown)
    }

    pub fn operation(
        &self,
        instance_id: &str,
        operation_id: &str,
    ) -> Option<&SaveProfileOperationRecord> {
        self.records
            .get(&(instance_id.to_owned(), operation_id.to_owned()))
    }

    pub fn selected(&self) -> Option<&SaveProfileId> {
        self.selected.as_ref()
    }

    pub fn set_authority(&mut self, authority: SaveProfileAuthority) {
        self.authority = authority;
    }

    pub fn forwarding_mut(&mut self) -> &mut F {
        &mut self.forwarding
    }

    pub fn into_store(self) -> S {
        self.store
    }

    fn dispatch(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileResult, SaveProfileLedgerError> {
        let key = key(&request);
        let forwarded = match self.forwarding.forward(request.clone()) {
            Ok(response) => response,
            Err(
                SaveProfileTransportFault::TimeoutAfterWrite
                | SaveProfileTransportFault::DisconnectedAfterWrite,
            ) => {
                let unknown = unknown_result(&request);
                self.persist_result(key, unknown.clone())?;
                return Ok(unknown);
            }
            Err(SaveProfileTransportFault::UnavailableBeforeWrite)
            | Err(SaveProfileTransportFault::RejectedBeforeWrite) => {
                let rejected = rejected_result(&request, SaveProfileStatus::Rejected);
                self.persist_result(key, rejected.clone())?;
                return Ok(rejected);
            }
            Err(error) => {
                let unknown = unknown_result(&request);
                self.persist_result(key, unknown.clone())?;
                return Err(SaveProfileLedgerError::Transport(error));
            }
        };
        let result = match validate_response(&request, forwarded) {
            Ok(result) => result,
            Err(error) => {
                let unknown = unknown_result(&request);
                self.persist_result(key, unknown)?;
                return Err(error);
            }
        };
        self.persist_result(key, result.clone())?;
        Ok(result)
    }

    fn persist_result(
        &mut self,
        key: (String, String),
        result: SaveProfileResult,
    ) -> Result<(), SaveProfileLedgerError> {
        let Some(mut record) = self.records.get(&key).cloned() else {
            return Err(SaveProfileLedgerError::OperationNotFound);
        };
        record.status = result.status;
        record.result = Some(result.clone());
        self.store.update(record.clone())?;
        self.records.insert(key, record);
        if result.route == SaveProfileRoute::Select {
            match result.status {
                SaveProfileStatus::Settled => {
                    self.selected = result.profile_id.clone();
                    self.pending_selection = None;
                }
                SaveProfileStatus::Rejected
                | SaveProfileStatus::Blocked
                | SaveProfileStatus::Cancelled => self.pending_selection = None,
                SaveProfileStatus::Accepted | SaveProfileStatus::Unknown => {}
            }
        }
        Ok(())
    }
}
