// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::path::Path;

use rusqlite::{Connection, ErrorCode, Row};

use super::super::recovery_types::{
    RecoveryBootContext, RecoveryBootState, RecoveryHostFence, RecoveryLease, RecoveryLeaseState,
    RecoveryReleaseSet, RecoveryStoreError,
};
use super::sql;

pub(super) fn open_private(path: &Path) -> Result<File, RecoveryStoreError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| RecoveryStoreError::Io(format!("recovery store open failed: {error}")))
}

pub(super) fn ensure_parent(path: &Path) -> Result<(), RecoveryStoreError> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            RecoveryStoreError::Io(format!("recovery store directory creation failed: {error}"))
        })?;
    }
    Ok(())
}

/// Authority locks are path based. Refuse symlink/reparse aliases before either
/// the lock or SQLite file is opened so a second spelling cannot acquire a
/// second authority namespace.
pub(super) fn reject_symlink_path(path: &Path) -> Result<(), RecoveryStoreError> {
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(RecoveryStoreError::InvalidInput(
                "recovery store paths may not contain symlink aliases".to_owned(),
            ));
        }
    }
    Ok(())
}

pub(super) fn migrate(conn: &Connection) -> Result<(), RecoveryStoreError> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(map_sql_error)?;
    if version != 0 && version != 1 && version != 2 && version != 3 {
        return Err(RecoveryStoreError::IncompatibleSchema {
            found: version,
            expected: 3,
        });
    }
    if version == 0 {
        conn.execute_batch(sql::CREATE_SCHEMA)
            .map_err(map_sql_error)?;
    } else if version == 1 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS admission_tickets (
                 ticket_id TEXT PRIMARY KEY,
                 operation_id TEXT NOT NULL,
                 instance_id TEXT NOT NULL,
                 payload_digest TEXT NOT NULL,
                 boot_id TEXT NOT NULL,
                 instance_incarnation TEXT NOT NULL,
                 lease_epoch INTEGER NOT NULL,
                 host_fence_id TEXT NOT NULL,
                 state TEXT NOT NULL,
                 issued_at_millis INTEGER NOT NULL,
                 expires_at_millis INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS admission_tickets_operation_idx
                 ON admission_tickets(instance_id, operation_id);
             CREATE TABLE IF NOT EXISTS operation_archive (
                 instance_id TEXT NOT NULL,
                 operation_id TEXT NOT NULL,
                 deployment_id TEXT NOT NULL,
                 instance_incarnation TEXT NOT NULL,
                 boot_id TEXT NOT NULL,
                 authority_generation INTEGER NOT NULL,
                 lease_id TEXT NOT NULL,
                 lease_epoch INTEGER NOT NULL,
                 state TEXT NOT NULL,
                 payload_digest TEXT NOT NULL,
                 schema_digest TEXT NOT NULL,
                 canonical_json BLOB NOT NULL,
                 expected_state_id TEXT NOT NULL,
                 expected_generation INTEGER NOT NULL,
                 catalog_digest TEXT NOT NULL,
                 response_status INTEGER,
                 response_body BLOB,
                 witness_json BLOB,
                 uncertainty_reason TEXT,
                 created_at_millis INTEGER NOT NULL,
                 updated_at_millis INTEGER NOT NULL,
                 archived_at_millis INTEGER NOT NULL,
                 PRIMARY KEY (instance_id, operation_id)
             );
             CREATE INDEX IF NOT EXISTS operation_archive_digest_idx
                 ON operation_archive(instance_id, operation_id, payload_digest);
             PRAGMA user_version = 2;",
        )
        .map_err(map_sql_error)?;
    }
    if version != 0 && version < 3 {
        conn.execute_batch(
            "ALTER TABLE leases ADD COLUMN host_fence_id TEXT;
             ALTER TABLE leases ADD COLUMN host_fence_generation INTEGER;
             ALTER TABLE leases ADD COLUMN host_installation_id TEXT;
             ALTER TABLE leases ADD COLUMN host_grant_digest TEXT;
             ALTER TABLE leases ADD COLUMN host_state TEXT NOT NULL DEFAULT 'UNINSTALLED';
             ALTER TABLE leases ADD COLUMN host_install_generation INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE leases ADD COLUMN host_renew_sequence INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE leases ADD COLUMN host_ack_message_id TEXT;
             ALTER TABLE leases ADD COLUMN host_ack_recorded_at_millis INTEGER;
             ALTER TABLE leases ADD COLUMN pending_expires_at_millis INTEGER;
             ALTER TABLE leases ADD COLUMN pending_renew_sequence INTEGER;
             UPDATE leases SET host_state = 'UNINSTALLED' WHERE host_state IS NULL;
             PRAGMA user_version = 3;",
        )
        .map_err(map_sql_error)?;
    }
    Ok(())
}

pub(super) fn row_boot(row: &Row<'_>) -> rusqlite::Result<RecoveryBootContext> {
    Ok(RecoveryBootContext {
        deployment_id: row.get(0)?,
        instance_id: row.get(1)?,
        instance_incarnation: row.get(2)?,
        boot_id: row.get(3)?,
        authority_generation: row_u64(row, 4)?,
        release: RecoveryReleaseSet::new(
            &row.get::<_, String>(5)?,
            &row.get::<_, String>(6)?,
            &row.get::<_, String>(7)?,
            &row.get::<_, String>(8)?,
        )
        .map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at_millis: row_u64(row, 9)?,
        state: RecoveryBootState::parse(&row.get::<_, String>(10)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

pub(super) fn row_fence(row: &Row<'_>) -> rusqlite::Result<RecoveryHostFence> {
    Ok(RecoveryHostFence {
        host_fence_id: row.get(0)?,
        deployment_id: row.get(1)?,
        instance_id: row.get(2)?,
        instance_incarnation: row.get(3)?,
        boot_id: row.get(4)?,
        authority_generation: row_u64(row, 5)?,
        fence_generation: row_u64(row, 6)?,
        created_at_millis: row_u64(row, 7)?,
    })
}

pub(super) fn row_lease(row: &Row<'_>, token: &str) -> rusqlite::Result<RecoveryLease> {
    Ok(RecoveryLease {
        deployment_id: row.get(0)?,
        instance_id: row.get(1)?,
        instance_incarnation: row.get(2)?,
        boot_id: row.get(3)?,
        authority_generation: row_u64(row, 4)?,
        lease_id: row.get(5)?,
        lease_epoch: row_u64(row, 6)?,
        fence_token: token.to_owned(),
        issued_at_millis: row_u64(row, 8)?,
        expires_at_millis: row_u64(row, 9)?,
        ttl_seconds: row_u64(row, 10)?,
        renewal_interval_seconds: row_u64(row, 11)?,
        last_renew_sequence: row_u64(row, 12)?,
    })
}

pub(crate) fn parse_lease_state(value: String) -> rusqlite::Result<RecoveryLeaseState> {
    match value.as_str() {
        "ACTIVE" => Ok(RecoveryLeaseState::Active),
        "EXPIRED" => Ok(RecoveryLeaseState::Expired),
        "REVOKED" => Ok(RecoveryLeaseState::Revoked),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(crate) fn row_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

pub(crate) fn map_sql_error(error: rusqlite::Error) -> RecoveryStoreError {
    if matches!(
        error,
        rusqlite::Error::SqliteFailure(ref inner, _)
            if matches!(inner.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        RecoveryStoreError::Busy
    } else {
        RecoveryStoreError::Sql(error.to_string())
    }
}
