// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::{
    LifecycleAction, LifecycleOperation, LifecycleOperationState, LifecycleOwnership,
    LifecycleRecordKey, LifecycleRecordStore, LifecycleStoreError,
};

#[derive(Clone, Debug, Default)]
pub struct InMemoryLifecycleStore {
    records: BTreeMap<LifecycleRecordKey, LifecycleOperation>,
    ownership: BTreeMap<super::InstanceId, LifecycleOwnership>,
    fail_insert: bool,
    fail_update: bool,
    fail_update_after: Option<usize>,
}

impl InMemoryLifecycleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fail_insert(&mut self, value: bool) {
        self.fail_insert = value;
    }

    pub fn set_fail_update(&mut self, value: bool) {
        self.fail_update = value;
    }

    pub fn set_fail_update_after(&mut self, updates_before_failure: Option<usize>) {
        self.fail_update_after = updates_before_failure;
    }
}

impl LifecycleRecordStore for InMemoryLifecycleStore {
    fn get(
        &self,
        key: LifecycleRecordKey,
    ) -> Result<Option<LifecycleOperation>, LifecycleStoreError> {
        Ok(self.records.get(&key).cloned())
    }

    fn insert(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        if self.fail_insert {
            return Err(LifecycleStoreError::Database);
        }
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        if self.records.contains_key(&key) {
            return Err(LifecycleStoreError::Conflict);
        }
        if operation.state().is_active() {
            let latest = self
                .records
                .values()
                .filter(|candidate| {
                    candidate.instance_id() == operation.instance_id()
                        && candidate.state() != LifecycleOperationState::Rejected
                })
                .max_by_key(|candidate| operation_order_key(candidate));
            let has_in_flight = latest.is_some_and(|candidate| {
                matches!(
                    candidate.state(),
                    LifecycleOperationState::IntentRecorded
                        | LifecycleOperationState::Starting
                        | LifecycleOperationState::Stopping
                        | LifecycleOperationState::Restarting
                )
            });
            let has_active = latest.is_some_and(|candidate| candidate.state().is_active());
            let launch = matches!(operation.action(), LifecycleAction::LaunchNew { .. });
            if has_in_flight
                || (launch && (has_active || self.ownership.contains_key(&operation.instance_id())))
            {
                return Err(LifecycleStoreError::Conflict);
            }
        }
        self.records.insert(key, operation);
        Ok(())
    }

    fn update(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        if self.fail_update
            || self
                .fail_update_after
                .is_some_and(|remaining| remaining == 0)
        {
            return Err(LifecycleStoreError::Database);
        }
        if let Some(remaining) = self.fail_update_after.as_mut() {
            *remaining = remaining.saturating_sub(1);
        }
        let key = LifecycleRecordKey::new(operation.instance_id(), operation.operation_id());
        if !self.records.contains_key(&key) {
            return Err(LifecycleStoreError::NotFound);
        }
        self.records.insert(key, operation);
        Ok(())
    }

    fn list(&self) -> Result<Vec<LifecycleOperation>, LifecycleStoreError> {
        Ok(self.records.values().cloned().collect())
    }

    fn count(&self) -> Result<usize, LifecycleStoreError> {
        Ok(self.records.len())
    }

    fn list_ownership(&self) -> Result<Vec<LifecycleOwnership>, LifecycleStoreError> {
        Ok(self.ownership.values().cloned().collect())
    }

    fn set_ownership(&mut self, ownership: LifecycleOwnership) -> Result<(), LifecycleStoreError> {
        let operation = self
            .records
            .get(&LifecycleRecordKey::new(
                ownership.instance_id(),
                ownership.operation_id(),
            ))
            .ok_or(LifecycleStoreError::Conflict)?;
        if !operation.state().is_active() {
            return Err(LifecycleStoreError::Conflict);
        }
        if let Some(current) = self.ownership.get(&ownership.instance_id()) {
            if current.operation_id() != ownership.operation_id() {
                if matches!(operation.action(), LifecycleAction::LaunchNew { .. })
                    || !operation.state().is_active()
                    || !ownership_precedes(current, &ownership)
                {
                    return Err(LifecycleStoreError::Conflict);
                }
            } else if ownership.sequence() < current.sequence() {
                return Err(LifecycleStoreError::Conflict);
            }
        }
        self.ownership.insert(ownership.instance_id(), ownership);
        Ok(())
    }

    fn clear_ownership_if(
        &mut self,
        ownership: &LifecycleOwnership,
    ) -> Result<(), LifecycleStoreError> {
        match self.ownership.get(&ownership.instance_id()) {
            None => Ok(()),
            Some(current) if current == ownership => {
                self.ownership.remove(&ownership.instance_id());
                Ok(())
            }
            Some(_) => Err(LifecycleStoreError::Conflict),
        }
    }

    fn clear_ownership(
        &mut self,
        instance_id: super::InstanceId,
    ) -> Result<(), LifecycleStoreError> {
        self.ownership.remove(&instance_id);
        Ok(())
    }
}

fn ownership_precedes(current: &LifecycleOwnership, candidate: &LifecycleOwnership) -> bool {
    match (current.sequence(), candidate.sequence()) {
        (0, 0) => candidate.operation_id().value() > current.operation_id().value(),
        (0, _) => true,
        (_, 0) => false,
        (current, candidate) => candidate > current,
    }
}

fn operation_order_key(operation: &LifecycleOperation) -> (u8, u64, u64) {
    if operation.sequence() == 0 {
        (0, operation.operation_id().value(), 0)
    } else {
        (1, operation.sequence(), operation.operation_id().value())
    }
}
