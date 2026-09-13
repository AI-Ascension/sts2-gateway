// SPDX-License-Identifier: MIT

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use rusqlite::{Connection, Error as SqlError, ErrorCode};
use uuid::Uuid;

use crate::InstanceId;

use super::{
    LifecycleOperation, LifecycleOwnership, LifecycleRecordKey, LifecycleRecordStore,
    LifecycleStoreError,
};

#[path = "process_store_sqlite_admission.rs"]
mod admission;
#[path = "process_store_sqlite_records.rs"]
mod records;

/// SQLite-backed operation records. Each mutation is committed before the process port is called.
pub struct SqliteLifecycleStore {
    connection: Connection,
    /// Held for the lifetime of the store so only one lifecycle coordinator
    /// can admit effects against a given on-disk journal. SQLite's row-level
    /// conflict handling alone cannot fence coordinators that cache records
    /// and ownership in memory.
    _coordinator_lock: Option<File>,
    /// Durable coordinator token used to fence a stale store if its lock file
    /// is replaced or unlinked while the original coordinator is still alive.
    coordinator_token: [u8; 16],
}

impl SqliteLifecycleStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LifecycleStoreError> {
        let path = path.as_ref();
        let connection = Connection::open(path).map_err(|_| LifecycleStoreError::Database)?;
        if path == Path::new(":memory:") {
            return Self::from_connection(connection, None);
        }
        let lock_path = Self::lock_path(path);
        let lock = Self::acquire_lock(&lock_path)?;
        Self::from_connection(connection, Some(lock))
    }

    pub fn open_in_memory() -> Result<Self, LifecycleStoreError> {
        let connection = Connection::open_in_memory().map_err(|_| LifecycleStoreError::Database)?;
        Self::from_connection(connection, None)
    }

    fn from_connection(
        connection: Connection,
        coordinator_lock: Option<File>,
    ) -> Result<Self, LifecycleStoreError> {
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
                 );
                 CREATE TABLE IF NOT EXISTS lifecycle_coordinator (
                   singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                   token BLOB NOT NULL
                 );",
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        let coordinator_token = *Uuid::new_v4().as_bytes();
        connection
            .execute(
                "INSERT INTO lifecycle_coordinator (singleton, token)
                 VALUES (1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET token = excluded.token",
                rusqlite::params![coordinator_token.as_slice()],
            )
            .map_err(|_| LifecycleStoreError::Database)?;
        Ok(Self {
            connection,
            _coordinator_lock: coordinator_lock,
            coordinator_token,
        })
    }

    fn lock_path(path: &Path) -> PathBuf {
        // `open` has already created an absent database file, so canonicalize
        // now and ensure aliases (for example a symlink and its target) share
        // one lock file.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut lock_path = canonical.into_os_string();
        lock_path.push(".lifecycle.lock");
        PathBuf::from(lock_path)
    }

    fn acquire_lock(path: &Path) -> Result<File, LifecycleStoreError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| LifecycleStoreError::Database)?;
        match lock.try_lock_exclusive() {
            Ok(()) => Ok(lock),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                Err(LifecycleStoreError::Conflict)
            }
            Err(_) => Err(LifecycleStoreError::Database),
        }
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
