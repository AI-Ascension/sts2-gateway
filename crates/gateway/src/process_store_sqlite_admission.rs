// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, Transaction};

use super::super::{LifecycleAction, LifecycleOperationState};
use super::{LifecycleOperation, LifecycleOwnership, LifecycleStoreError, SqliteLifecycleStore};

pub(super) fn ensure_token(
    transaction: &Transaction<'_>,
    token: &[u8; 16],
) -> Result<(), LifecycleStoreError> {
    let stored = transaction
        .query_row(
            "SELECT token FROM lifecycle_coordinator WHERE singleton = 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(|_| LifecycleStoreError::Database)?;
    if stored.as_slice() != token {
        return Err(LifecycleStoreError::Conflict);
    }
    Ok(())
}

pub(super) fn ensure_token_read(store: &SqliteLifecycleStore) -> Result<(), LifecycleStoreError> {
    let stored = store
        .connection
        .query_row(
            "SELECT token FROM lifecycle_coordinator WHERE singleton = 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .map_err(|_| LifecycleStoreError::Database)?;
    if stored.as_slice() != store.coordinator_token {
        return Err(LifecycleStoreError::Conflict);
    }
    Ok(())
}

fn decode_operation_rows(
    transaction: &Transaction<'_>,
    instance_id: i64,
) -> Result<Vec<LifecycleOperation>, LifecycleStoreError> {
    let mut statement = transaction
        .prepare(
            "SELECT body FROM lifecycle_operations
             WHERE instance_id = ?1",
        )
        .map_err(|_| LifecycleStoreError::Database)?;
    let rows = statement
        .query_map(rusqlite::params![instance_id], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|_| LifecycleStoreError::Database)?;
    rows.map(|row| {
        row.map_err(|_| LifecycleStoreError::Database)
            .and_then(|bytes| SqliteLifecycleStore::decode(&bytes))
    })
    .collect()
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

pub(super) fn can_insert_operation(
    transaction: &Transaction<'_>,
    operation: &LifecycleOperation,
    instance_id: i64,
) -> Result<bool, LifecycleStoreError> {
    if !operation.state().is_active() {
        return Ok(true);
    }
    let records = decode_operation_rows(transaction, instance_id)?;
    let latest = records
        .iter()
        .filter(|candidate| candidate.state() != LifecycleOperationState::Rejected)
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
    if has_in_flight {
        return Ok(false);
    }
    if !matches!(operation.action(), LifecycleAction::LaunchNew { .. }) {
        return Ok(true);
    }
    let has_active = latest.is_some_and(|candidate| candidate.state().is_active());
    let has_ownership = transaction
        .query_row(
            "SELECT 1 FROM lifecycle_ownership WHERE instance_id = ?1",
            rusqlite::params![instance_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| LifecycleStoreError::Database)?
        .is_some();
    Ok(!has_active && !has_ownership)
}

pub(super) fn ownership_update_allowed(
    transaction: &Transaction<'_>,
    ownership: &LifecycleOwnership,
    current: Option<&LifecycleOwnership>,
) -> Result<(), LifecycleStoreError> {
    let instance_id = SqliteLifecycleStore::sql_id(ownership.instance_id().value())?;
    let operation = transaction
        .query_row(
            "SELECT body FROM lifecycle_operations
             WHERE instance_id = ?1 AND operation_id = ?2",
            rusqlite::params![
                instance_id,
                SqliteLifecycleStore::sql_id(ownership.operation_id().value())?
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(|_| LifecycleStoreError::Database)?
        .map(|body| SqliteLifecycleStore::decode(&body))
        .transpose()?
        .ok_or(LifecycleStoreError::Conflict)?;
    if !operation.state().is_active() {
        return Err(LifecycleStoreError::Conflict);
    }
    if let Some(current) = current {
        if current.operation_id() != ownership.operation_id()
            && (matches!(operation.action(), LifecycleAction::LaunchNew { .. })
                || !ownership_precedes(current, ownership))
        {
            return Err(LifecycleStoreError::Conflict);
        }
        if current.operation_id() == ownership.operation_id()
            && ownership.sequence() < current.sequence()
        {
            return Err(LifecycleStoreError::Conflict);
        }
    }
    Ok(())
}
