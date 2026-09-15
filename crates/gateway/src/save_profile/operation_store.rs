// SPDX-License-Identifier: MIT

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs2::FileExt;
use rusqlite::{Connection, TransactionBehavior, params};
use uuid::Uuid;

use super::ledger_types::{
    SaveProfileLedgerError as Error, SaveProfileOperationRecord, SaveProfileRecordStore,
};
use super::operation_record::{MAX_RECORD_BYTES, decode, encode, validate_update};

/// Maximum persisted forwarded-operation records in one journal.
pub const SAVE_PROFILE_OPERATION_CAPACITY: usize = 256;

/// Durable forwarded save-profile intents/results behind the existing ledger port.
///
/// Uses a separate gateway-private journal, not the allocation store. Opening rejects ephemeral
/// paths; every write is fenced inside an immediate transaction. No runtime route is enabled.
/// A deployment must exclusively own the database directory and preserve its database/lock files.
pub struct SqliteSaveProfileOperationStore {
    connection: Connection,
    _lock: File,
    token: [u8; 16],
}

impl SqliteSaveProfileOperationStore {
    /// Opens a gateway-owned persistent journal. Invalid/corrupt storage fails closed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let spelling = path.as_os_str().to_string_lossy();
        if spelling.is_empty() || spelling == ":memory:" || spelling.starts_with("file:") {
            return Err(Error::PersistenceFailed);
        }
        let mut connection = Connection::open(path).map_err(db_error)?;
        let canonical = path.canonicalize().map_err(|_| Error::PersistenceFailed)?;
        let mut lock_path = canonical.into_os_string();
        lock_path.push(".save-profile-operations.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| Error::PersistenceFailed)?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::OperationConflict)?;
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(db_error)?;
        let token = *Uuid::new_v4().as_bytes();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        super::operation_schema::initialize(&transaction)?;
        // Validate existing rows before taking ownership, so corrupt data never becomes authority.
        read_records(&transaction)?;
        let changed = transaction
            .execute(
                "INSERT INTO save_profile_operation_owner VALUES(1,?1)
             ON CONFLICT(singleton) DO UPDATE SET token=excluded.token",
                [token.as_slice()],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(Error::PersistenceFailed);
        }
        ensure_owner(&transaction, &token)?;
        transaction.commit().map_err(db_error)?;
        Ok(Self {
            connection,
            _lock: lock,
            token,
        })
    }
}

fn db_error(_: rusqlite::Error) -> Error {
    Error::PersistenceFailed
}

fn ensure_owner(connection: &Connection, token: &[u8; 16]) -> Result<(), Error> {
    super::operation_schema::verify(connection)?;
    let stored: Vec<u8> = connection
        .query_row(
            "SELECT token FROM save_profile_operation_owner WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if stored.as_slice() != token {
        return Err(Error::OperationConflict);
    }
    Ok(())
}

fn read_records(connection: &Connection) -> Result<Vec<SaveProfileOperationRecord>, Error> {
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM save_profile_operation_records",
            [],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if count > SAVE_PROFILE_OPERATION_CAPACITY as i64 {
        return Err(Error::CapacityExceeded);
    }
    let mut statement = connection.prepare(
        "SELECT instance_id,operation_id,length(body),body,length(instance_id),length(operation_id)
         FROM save_profile_operation_records ORDER BY sequence DESC"
    ).map_err(db_error)?;
    let mut rows = statement.query([]).map_err(db_error)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        let length: i64 = row.get(2).map_err(db_error)?;
        let instance_length: i64 = row.get(4).map_err(db_error)?;
        let operation_length: i64 = row.get(5).map_err(db_error)?;
        if length < 0
            || length > MAX_RECORD_BYTES as i64
            || !(1..=128).contains(&instance_length)
            || !(1..=128).contains(&operation_length)
        {
            return Err(Error::PersistenceFailed);
        }
        let body: Vec<u8> = row.get(3).map_err(db_error)?;
        let record = decode(&body)?;
        let instance: String = row.get(0).map_err(db_error)?;
        let operation: String = row.get(1).map_err(db_error)?;
        if instance != record.request.context.instance_id || operation != record.operation_id {
            return Err(Error::PersistenceFailed);
        }
        if records
            .first()
            .is_some_and(|first: &SaveProfileOperationRecord| {
                first.request.context.instance_id != record.request.context.instance_id
            })
        {
            return Err(Error::PersistenceFailed);
        }
        records.push(record);
    }
    Ok(records)
}

impl SaveProfileRecordStore for SqliteSaveProfileOperationStore {
    fn list(&mut self) -> Result<Vec<SaveProfileOperationRecord>, Error> {
        let transaction = self.connection.transaction().map_err(db_error)?;
        ensure_owner(&transaction, &self.token)?;
        let records = read_records(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(records)
    }

    fn insert(&mut self, record: SaveProfileOperationRecord) -> Result<(), Error> {
        let body = encode(&record)?;
        if record.status != super::SaveProfileStatus::Accepted || record.result.is_some() {
            return Err(Error::InvalidRequest);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_owner(&transaction, &self.token)?;
        let records = read_records(&transaction)?;
        if records
            .iter()
            .any(|old| old.request.context.instance_id != record.request.context.instance_id)
        {
            return Err(Error::OperationConflict);
        }
        if records.iter().any(|old| {
            old.operation_id == record.operation_id
                && old.request.context.instance_id == record.request.context.instance_id
        }) {
            return Err(Error::OperationConflict);
        }
        if records.len() >= SAVE_PROFILE_OPERATION_CAPACITY {
            return Err(Error::CapacityExceeded);
        }
        let changed = transaction
            .execute(
                "INSERT INTO save_profile_operation_records(instance_id,operation_id,body)
             VALUES(?1,?2,?3)",
                params![
                    record.request.context.instance_id,
                    record.operation_id,
                    body
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(Error::PersistenceFailed);
        }
        verify_write(&transaction, &record, &body)?;
        transaction.commit().map_err(db_error)
    }

    fn update(&mut self, record: SaveProfileOperationRecord) -> Result<(), Error> {
        let body = encode(&record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        ensure_owner(&transaction, &self.token)?;
        let previous = read_records(&transaction)?
            .into_iter()
            .find(|old| {
                old.operation_id == record.operation_id
                    && old.request.context.instance_id == record.request.context.instance_id
            })
            .ok_or(Error::OperationNotFound)?;
        validate_update(&previous, &record)?;
        let changed = transaction
            .execute(
                "UPDATE save_profile_operation_records SET body=?3
             WHERE instance_id=?1 AND operation_id=?2",
                params![
                    record.request.context.instance_id,
                    record.operation_id,
                    body
                ],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(Error::OperationNotFound);
        }
        verify_write(&transaction, &record, &body)?;
        transaction.commit().map_err(db_error)
    }
}

fn verify_write(
    connection: &Connection,
    record: &SaveProfileOperationRecord,
    body: &[u8],
) -> Result<(), Error> {
    let actual: Vec<u8> = connection.query_row(
        "SELECT body FROM save_profile_operation_records WHERE instance_id=?1 AND operation_id=?2",
        params![record.request.context.instance_id, record.operation_id], |row| row.get(0),
    ).map_err(db_error)?;
    if actual != body {
        return Err(Error::PersistenceFailed);
    }
    Ok(())
}
