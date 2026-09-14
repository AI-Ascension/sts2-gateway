// SPDX-License-Identifier: MIT

//! Durable (SQLite) store for isolated user-data provisioning records.
//!
//! The provisioner persists an intent record before it touches the allocation port, so a process
//! restart must recover the same opaque identity from stable storage rather than an in-memory map.
//! This adapter mirrors the lifecycle store: each mutation is committed before the caller proceeds,
//! and only one process may own a given on-disk journal.

use std::path::Path;

use rusqlite::{Connection, params};

use super::provisioning_types::{
    UserDataProvisioningError, UserDataProvisioningRecord, UserDataRecordStore,
};

/// SQLite-backed record store for [`UserDataProvisioningRecord`]s.
pub struct SqliteUserDataRecordStore {
    connection: Connection,
}

impl SqliteUserDataRecordStore {
    /// Open (or create) the durable store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, UserDataProvisioningError> {
        let connection = Connection::open(path.as_ref())
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Self::from_connection(connection)
    }

    /// Open an ephemeral store used by component tests.
    pub fn open_in_memory() -> Result<Self, UserDataProvisioningError> {
        let connection = Connection::open_in_memory()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, UserDataProvisioningError> {
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 CREATE TABLE IF NOT EXISTS save_profile_user_data_records (
                   instance_id TEXT NOT NULL,
                   operation_id TEXT NOT NULL,
                   body BLOB NOT NULL,
                   PRIMARY KEY (instance_id, operation_id)
                 );",
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Ok(Self { connection })
    }

    fn encode(record: &UserDataProvisioningRecord) -> Result<Vec<u8>, UserDataProvisioningError> {
        serde_json::to_vec(record).map_err(|_| UserDataProvisioningError::PersistenceFailed)
    }

    fn decode(body: Vec<u8>) -> Result<UserDataProvisioningRecord, UserDataProvisioningError> {
        serde_json::from_slice(&body).map_err(|_| UserDataProvisioningError::PersistenceFailed)
    }
}

impl UserDataRecordStore for SqliteUserDataRecordStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM save_profile_user_data_records ORDER BY instance_id, operation_id")
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        let mut records = Vec::new();
        for row in rows {
            let body = row.map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
            records.push(Self::decode(body)?);
        }
        Ok(records)
    }

    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let body = Self::encode(&record)?;
        let changed = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO save_profile_user_data_records
                   (instance_id, operation_id, body) VALUES (?1, ?2, ?3)",
                params![record.context.instance_id, record.operation_id, body],
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        if changed == 0 {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        Ok(())
    }

    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let body = Self::encode(&record)?;
        let changed = self
            .connection
            .execute(
                "UPDATE save_profile_user_data_records SET body = ?3
                   WHERE instance_id = ?1 AND operation_id = ?2",
                params![record.context.instance_id, record.operation_id, body],
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        if changed == 0 {
            return Err(UserDataProvisioningError::OperationNotFound);
        }
        Ok(())
    }
}
