// SPDX-License-Identifier: MIT

//! Durable (SQLite) store for isolated user-data provisioning records.
//!
//! The provisioner persists an intent record before it touches the allocation port, so a process
//! restart must recover the same opaque identity from stable storage rather than an in-memory map.
//! This adapter mirrors the lifecycle store: each mutation is committed before the caller proceeds,
//! and only one coordinator may own a given on-disk journal.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::coordinator_lock::CoordinatorLock;
use rusqlite::{Connection, TransactionBehavior, params};
use uuid::Uuid;

use super::durable_store_query::{db_error, read_records, validate_owner_row, verify_write};
use super::provisioning_record::{MAX_ALLOCATIONS, encode, validate_update};
use super::provisioning_types::{
    UserDataProvisioningError, UserDataProvisioningRecord, UserDataRecordStore,
};

/// SQLite-backed record store for [`UserDataProvisioningRecord`]s.
pub struct SqliteUserDataRecordStore {
    connection: Connection,
    /// Held for the lifetime of the store so only one coordinator can allocate against a given
    /// on-disk journal. The provisioner caches `next_identity` in memory, so two live stores would
    /// otherwise both reserve the same identity for different operations.
    _journal_lock: Option<CoordinatorLock>,
    /// Durable token that fences a stale store if its lock file is replaced or unlinked while the
    /// original coordinator is still alive.
    coordinator_token: [u8; 16],
}

impl SqliteUserDataRecordStore {
    /// Open (or create) the durable store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, UserDataProvisioningError> {
        let path = path.as_ref();
        // An empty path makes SQLite open a transient database, which would silently accept
        // provisioning intents that do not survive the connection.
        if path.as_os_str().is_empty() || path == Path::new(":memory:") {
            return Err(UserDataProvisioningError::PersistenceFailed);
        }
        // `Connection::open` enables SQLite URI handling, so a `file:...` spelling would resolve to
        // the same database while defeating the lock-file derivation below. Reject it so one
        // database has exactly one owner.
        if path.as_os_str().to_string_lossy().starts_with("file:") {
            return Err(UserDataProvisioningError::PersistenceFailed);
        }
        let connection =
            Connection::open(path).map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        let lock = Self::acquire_lock(&Self::lock_path(path))?;
        Self::from_connection(connection, Some(lock))
    }

    /// Open an ephemeral store used by component tests.
    pub fn open_in_memory() -> Result<Self, UserDataProvisioningError> {
        let connection = Connection::open_in_memory()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Self::from_connection(connection, None)
    }

    fn from_connection(
        connection: Connection,
        journal_lock: Option<CoordinatorLock>,
    ) -> Result<Self, UserDataProvisioningError> {
        // Keep connection teardown before authority release on every initialization path.
        let mut store = Self {
            connection,
            _journal_lock: journal_lock,
            coordinator_token: *Uuid::new_v4().as_bytes(),
        };
        super::durable_store_schema::verify_or_empty(&store.connection)?;
        store
            .connection
            .busy_timeout(std::time::Duration::from_millis(100))
            .map_err(db_error)?;
        store
            .connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(db_error)?;
        let transaction = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        super::durable_store_schema::initialize(&transaction)?;
        validate_owner_row(&transaction)?;
        read_records(&transaction)?;
        transaction
            .execute(
                "INSERT INTO save_profile_coordinator (singleton, token)
                 VALUES (1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET token = excluded.token",
                params![store.coordinator_token.as_slice()],
            )
            .map_err(db_error)?;
        Self::ensure_token(&transaction, &store.coordinator_token)?;
        transaction.commit().map_err(db_error)?;
        Ok(store)
    }

    fn lock_path(path: &Path) -> PathBuf {
        // `open` has already created an absent database file, so canonicalize now and ensure
        // aliases (for example a symlink and its target) share one lock file.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut lock_path = canonical.into_os_string();
        lock_path.push(".save-profile.lock");
        PathBuf::from(lock_path)
    }

    fn acquire_lock(path: &Path) -> Result<CoordinatorLock, UserDataProvisioningError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        match CoordinatorLock::acquire(lock) {
            Ok(lock) => Ok(lock),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                Err(UserDataProvisioningError::OperationConflict)
            }
            Err(_) => Err(UserDataProvisioningError::PersistenceFailed),
        }
    }

    fn ensure_token(
        connection: &Connection,
        token: &[u8; 16],
    ) -> Result<(), UserDataProvisioningError> {
        super::durable_store_schema::verify(connection)?;
        let count: i64 = connection
            .query_row("SELECT count(*) FROM save_profile_coordinator", [], |row| {
                row.get(0)
            })
            .map_err(db_error)?;
        if count != 1 {
            return Err(UserDataProvisioningError::PersistenceFailed);
        }
        let stored = connection
            .query_row(
                "SELECT token FROM save_profile_coordinator
                 WHERE singleton=1 AND typeof(token)='blob' AND length(token)=16",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        if stored.as_slice() != token {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        Ok(())
    }
}

impl UserDataRecordStore for SqliteUserDataRecordStore {
    fn list(&mut self) -> Result<Vec<UserDataProvisioningRecord>, UserDataProvisioningError> {
        let transaction = self.connection.transaction().map_err(db_error)?;
        Self::ensure_token(&transaction, &self.coordinator_token)?;
        let records = read_records(&transaction)?;
        transaction.commit().map_err(db_error)?;
        Ok(records)
    }

    fn insert(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let body = encode(&record)?;
        if record.status != super::UserDataProvisioningStatus::Pending {
            return Err(UserDataProvisioningError::InvalidRequest);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Self::ensure_token(&transaction, &self.coordinator_token)?;
        let records = read_records(&transaction)?;
        if records.iter().any(|old| {
            old.descriptor.identity == record.descriptor.identity
                || (old.context.instance_id == record.context.instance_id
                    && old.operation_id == record.operation_id)
        }) {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        if records.len() >= MAX_ALLOCATIONS {
            return Err(UserDataProvisioningError::CapacityExceeded);
        }
        let changed = transaction
            .execute(
                "INSERT OR IGNORE INTO save_profile_user_data_records
                   (instance_id, operation_id, body) VALUES (?1, ?2, ?3)",
                params![record.context.instance_id, record.operation_id, body],
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        if changed == 0 {
            return Err(UserDataProvisioningError::OperationConflict);
        }
        verify_write(&transaction, &record, &body)?;
        transaction
            .commit()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)
    }

    fn update(
        &mut self,
        record: UserDataProvisioningRecord,
    ) -> Result<(), UserDataProvisioningError> {
        let body = encode(&record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        Self::ensure_token(&transaction, &self.coordinator_token)?;
        let previous = read_records(&transaction)?
            .into_iter()
            .find(|old| {
                old.context.instance_id == record.context.instance_id
                    && old.operation_id == record.operation_id
            })
            .ok_or(UserDataProvisioningError::OperationNotFound)?;
        validate_update(&previous, &record)?;
        let changed = transaction
            .execute(
                "UPDATE save_profile_user_data_records SET body = ?3
                   WHERE instance_id = ?1 AND operation_id = ?2",
                params![record.context.instance_id, record.operation_id, body],
            )
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)?;
        if changed == 0 {
            return Err(UserDataProvisioningError::OperationNotFound);
        }
        verify_write(&transaction, &record, &body)?;
        transaction
            .commit()
            .map_err(|_| UserDataProvisioningError::PersistenceFailed)
    }
}
