// SPDX-License-Identifier: MIT

use std::path::Path;

use rusqlite::{Connection, Error as SqlError, ErrorCode, OptionalExtension};

use crate::InstanceId;

use super::{
    LifecycleOperation, LifecycleOwnership, LifecycleRecordKey, LifecycleRecordStore,
    LifecycleStoreError,
};

/// SQLite-backed operation records. Each mutation is committed before the process port is called.
pub struct SqliteLifecycleStore {
    connection: Connection,
}

impl SqliteLifecycleStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LifecycleStoreError> {
        let connection = Connection::open(path).map_err(|_| LifecycleStoreError::Database)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, LifecycleStoreError> {
        let connection = Connection::open_in_memory().map_err(|_| LifecycleStoreError::Database)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, LifecycleStoreError> {
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 CREATE TABLE IF NOT EXISTS lifecycle_operations (
                   instance_id INTEGER NOT NULL,
                   operation_id INTEGER NOT NULL,
                   body BLOB NOT NULL,
                   PRIMARY KEY (instance_id, operation_id)
                 );
                 CREATE TABLE IF NOT EXISTS lifecycle_ownership (
                   instance_id INTEGER PRIMARY KEY,
                   body BLOB NOT NULL
                 );",
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        Ok(Self { connection })
    }

    fn encode(operation: &LifecycleOperation) -> Result<Vec<u8>, LifecycleStoreError> {
        serde_json::to_vec(operation).map_err(|_| LifecycleStoreError::Serialization)
    }

    fn decode(bytes: &[u8]) -> Result<LifecycleOperation, LifecycleStoreError> {
        serde_json::from_slice(bytes).map_err(|_| LifecycleStoreError::Serialization)
    }

    fn encode_ownership(ownership: &LifecycleOwnership) -> Result<Vec<u8>, LifecycleStoreError> {
        serde_json::to_vec(ownership).map_err(|_| LifecycleStoreError::Serialization)
    }

    fn decode_ownership(bytes: &[u8]) -> Result<LifecycleOwnership, LifecycleStoreError> {
        serde_json::from_slice(bytes).map_err(|_| LifecycleStoreError::Serialization)
    }

    fn sql_id(value: u64) -> Result<i64, LifecycleStoreError> {
        i64::try_from(value).map_err(|_| LifecycleStoreError::Serialization)
    }

    fn insert_error(error: SqlError) -> LifecycleStoreError {
        if matches!(
            error,
            SqlError::SqliteFailure(ref failure, _)
                if failure.code == ErrorCode::ConstraintViolation
        ) {
            LifecycleStoreError::Conflict
        } else {
            LifecycleStoreError::Database
        }
    }
}

impl LifecycleRecordStore for SqliteLifecycleStore {
    fn get(
        &self,
        key: LifecycleRecordKey,
    ) -> Result<Option<LifecycleOperation>, LifecycleStoreError> {
        let instance_id = Self::sql_id(key.instance_id().value())?;
        let operation_id = Self::sql_id(key.operation_id().value())?;
        self.connection
            .query_row(
                "SELECT body FROM lifecycle_operations WHERE instance_id = ?1 AND operation_id = ?2",
                rusqlite::params![instance_id, operation_id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|_| LifecycleStoreError::Database)?
            .map(|bytes| Self::decode(&bytes))
            .transpose()
    }

    fn insert(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode(&operation)?;
        let instance_id = Self::sql_id(operation.instance_id().value())?;
        let operation_id = Self::sql_id(operation.operation_id().value())?;
        self.connection
            .execute(
                "INSERT INTO lifecycle_operations (instance_id, operation_id, body)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![instance_id, operation_id, bytes],
            )
            .map(|_| ())
            .map_err(Self::insert_error)
    }

    fn update(&mut self, operation: LifecycleOperation) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode(&operation)?;
        let instance_id = Self::sql_id(operation.instance_id().value())?;
        let operation_id = Self::sql_id(operation.operation_id().value())?;
        let changed = self
            .connection
            .execute(
                "UPDATE lifecycle_operations SET body = ?1
                 WHERE instance_id = ?2 AND operation_id = ?3",
                rusqlite::params![bytes, instance_id, operation_id],
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        if changed != 1 {
            return Err(LifecycleStoreError::NotFound);
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<LifecycleOperation>, LifecycleStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM lifecycle_operations ORDER BY instance_id, operation_id")
            .map_err(|_| LifecycleStoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|_| LifecycleStoreError::Database)?;
        rows.map(|row| {
            row.map_err(|_| LifecycleStoreError::Database)
                .and_then(|bytes| Self::decode(&bytes))
        })
        .collect()
    }

    fn count(&self) -> Result<usize, LifecycleStoreError> {
        self.connection
            .query_row("SELECT COUNT(*) FROM lifecycle_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| LifecycleStoreError::Database)
            .and_then(|count| {
                usize::try_from(count).map_err(|_| LifecycleStoreError::Serialization)
            })
    }

    fn list_ownership(&self) -> Result<Vec<LifecycleOwnership>, LifecycleStoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM lifecycle_ownership ORDER BY instance_id")
            .map_err(|_| LifecycleStoreError::Database)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|_| LifecycleStoreError::Database)?;
        rows.map(|row| {
            row.map_err(|_| LifecycleStoreError::Database)
                .and_then(|bytes| Self::decode_ownership(&bytes))
        })
        .collect()
    }

    fn set_ownership(&mut self, ownership: LifecycleOwnership) -> Result<(), LifecycleStoreError> {
        let bytes = Self::encode_ownership(&ownership)?;
        let instance_id = Self::sql_id(ownership.instance_id().value())?;
        self.connection
            .execute(
                "INSERT INTO lifecycle_ownership (instance_id, body)
                 VALUES (?1, ?2)
                 ON CONFLICT(instance_id) DO UPDATE SET body = excluded.body",
                rusqlite::params![instance_id, bytes],
            )
            .map(|_| ())
            .map_err(|_| LifecycleStoreError::Database)
    }

    fn clear_ownership(&mut self, instance_id: InstanceId) -> Result<(), LifecycleStoreError> {
        let instance_id = Self::sql_id(instance_id.value())?;
        self.connection
            .execute(
                "DELETE FROM lifecycle_ownership WHERE instance_id = ?1",
                rusqlite::params![instance_id],
            )
            .map(|_| ())
            .map_err(|_| LifecycleStoreError::Database)
    }
}
