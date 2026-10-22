// SPDX-License-Identifier: MIT

use super::configuration::{exact_path, private_directory};
use super::{GenerationRecord, LaunchProfile, NativeProcessError, ProcessOperationGeneration};
use crate::coordinator_lock::CoordinatorLock;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::fs::OpenOptions;
use std::path::Path;
use std::time::Duration;

#[path = "process_native_store_codec.rs"]
mod codec;
#[path = "process_native_store_format.rs"]
mod format;
use codec::{decode_row, encode};

pub(super) struct GenerationRegistry {
    connection: Connection,
    _lock: CoordinatorLock,
    capacity: usize,
    identity: String,
}

impl GenerationRegistry {
    pub(super) fn open(path: &Path, capacity: usize) -> Result<Self, NativeProcessError> {
        if capacity == 0 || capacity > 1024 {
            return Err(NativeProcessError::Registry);
        }
        let parent = path.parent().ok_or(NativeProcessError::Registry)?;
        exact_path(parent, true)?;
        private_directory(parent)?;
        if path.exists() {
            exact_path(path, false)?;
        }
        let lock_path = path.with_extension("process-lock");
        if lock_path.exists() {
            exact_path(&lock_path, false)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| NativeProcessError::Registry)?;
        let lock = CoordinatorLock::acquire(lock).map_err(|_| NativeProcessError::Registry)?;
        let connection = Connection::open(path).map_err(db_error)?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(db_error)?;
        let identity = format::initialize(&connection)?;
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(db_error)?;
        if integrity != "ok" {
            return Err(NativeProcessError::Registry);
        }
        let registry = Self {
            connection,
            _lock: lock,
            capacity,
            identity,
        };
        let count: i64 = registry
            .connection
            .query_row("SELECT COUNT(*) FROM process_generations", [], |row| {
                row.get(0)
            })
            .map_err(db_error)?;
        if count < 0 || count as usize > capacity {
            return Err(NativeProcessError::Registry);
        }
        let mut statement = registry
            .connection
            .prepare("SELECT id,generation_key,record FROM process_generations")
            .map_err(db_error)?;
        let records = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(db_error)?;
        let mut units = std::collections::BTreeSet::new();
        for row in records {
            let record = decode_row(row.map_err(db_error)?)?;
            if !units.insert(record.unit) {
                return Err(NativeProcessError::Registry);
            }
        }
        drop(statement);
        Ok(registry)
    }

    pub(super) fn identity(&self) -> &str {
        &self.identity
    }

    pub(super) fn by_key(&self, key: &str) -> Result<Option<GenerationRecord>, NativeProcessError> {
        let record = self
            .connection
            .query_row(
                "SELECT id,generation_key,record FROM process_generations WHERE generation_key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        record.map(decode_row).transpose()
    }

    pub(super) fn by_handle(&self, handle: u64) -> Result<GenerationRecord, NativeProcessError> {
        let handle = i64::try_from(handle).map_err(|_| NativeProcessError::Registry)?;
        let record = self
            .connection
            .query_row(
                "SELECT id,generation_key,record FROM process_generations WHERE id=?1",
                [handle],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?
            .ok_or(NativeProcessError::Registry)?;
        decode_row(record)
    }

    pub(super) fn reserve(
        &mut self,
        key: &str,
        profile: LaunchProfile,
        digest: &str,
        boot: &str,
    ) -> Result<(GenerationRecord, bool), NativeProcessError> {
        if let Some(record) = self.by_key(key)? {
            return Ok((record, false));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM process_generations", [], |row| {
                row.get(0)
            })
            .map_err(db_error)?;
        if count < 0 || count as usize >= self.capacity {
            return Err(NativeProcessError::Registry);
        }
        {
            let mut statement = transaction
                .prepare("SELECT id,generation_key,record FROM process_generations")
                .map_err(db_error)?;
            let records = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(db_error)?;
            for row in records {
                let existing = decode_row(row.map_err(db_error)?)?;
                if existing.profile.user_data() == profile.user_data() && !existing.absent {
                    return Err(NativeProcessError::Registry);
                }
            }
        }
        let mut record = GenerationRecord {
            key: key.to_owned(),
            handle: 0,
            unit: format!("sts2-gateway-{}.service", uuid::Uuid::new_v4().simple()),
            boot_id: boot.to_owned(),
            profile,
            binding_digest: digest.to_owned(),
            dispatched: false,
            stopping: false,
            absent: false,
            identity: None,
            invocation: None,
            control_group: None,
        };
        transaction
            .execute(
                "INSERT INTO process_generations(generation_key,record) VALUES(?1,?2)",
                params![key, b"{}".as_slice()],
            )
            .map_err(db_error)?;
        record.handle = u64::try_from(transaction.last_insert_rowid())
            .map_err(|_| NativeProcessError::Registry)?;
        let bytes = encode(&record)?;
        transaction
            .execute(
                "UPDATE process_generations SET record=?1 WHERE generation_key=?2",
                params![bytes, key],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok((record, true))
    }

    pub(super) fn save(&mut self, record: &GenerationRecord) -> Result<(), NativeProcessError> {
        let before = self
            .by_key(&record.key)?
            .ok_or(NativeProcessError::Registry)?;
        if before.handle != record.handle
            || before.unit != record.unit
            || before.profile != record.profile
            || before.binding_digest != record.binding_digest
            || before.boot_id != record.boot_id
            || (before.dispatched && !record.dispatched)
            || (before.stopping && !record.stopping)
            || (before.absent && !record.absent)
            || (before.identity.is_some() && before.identity != record.identity)
            || (before.invocation.is_some() && before.invocation != record.invocation)
            || (before.control_group.is_some() && before.control_group != record.control_group)
        {
            return Err(NativeProcessError::Registry);
        }
        let count = self
            .connection
            .execute(
                "UPDATE process_generations SET record=?1 WHERE generation_key=?2",
                params![encode(record)?, record.key],
            )
            .map_err(db_error)?;
        if count != 1 {
            return Err(NativeProcessError::Registry);
        }
        Ok(())
    }
}

pub(super) fn generation_key(generation: &ProcessOperationGeneration) -> String {
    let kind = match generation.kind() {
        crate::ProcessGenerationKind::LaunchNew => 1,
        crate::ProcessGenerationKind::RestartReplacement => 2,
    };
    let fields = [
        generation.instance_id().value(),
        generation.operation_id().value(),
        generation.sequence(),
        generation.request_epoch().value(),
        generation.authority_epoch().value(),
        generation.profile_id().value(),
        kind,
    ];
    let mut bytes = Vec::with_capacity(80);
    bytes.extend_from_slice(b"sts2-process-generation-v1\0");
    for field in fields {
        bytes.extend_from_slice(&field.to_be_bytes());
    }
    super::configuration::hex_digest(&bytes)
}

fn db_error(_: rusqlite::Error) -> NativeProcessError {
    NativeProcessError::Registry
}
