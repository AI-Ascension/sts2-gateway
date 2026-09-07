// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::super::GatewayRecoveryStore;
use super::super::recovery_types::{
    RecoveryBootContext, RecoveryReleaseSet, RecoveryStoreError, validate_uuid, validate_wire,
};

const PROVENANCE_SUFFIX: &str = "gateway-recovery.meta";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BackupProvenance {
    source_path: String,
    deployment_id: String,
    instance_id: String,
    authority_generation: u64,
}

impl GatewayRecoveryStore {
    /// Produces an online SQLite backup without replacing an existing path.
    /// The owner lock remains held for the lifetime of this store, and the
    /// destination is fsynced before this method returns.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), RecoveryStoreError> {
        let destination = destination.as_ref();
        if destination == self.path() || destination.exists() {
            return Err(RecoveryStoreError::BackupExists);
        }
        super::support::reject_symlink_path(destination)?;
        ensure_backup_parent(destination)?;
        let authority = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        let source_path = self.path.canonicalize().map_err(|error| {
            RecoveryStoreError::Io(format!("backup source canonicalization failed: {error}"))
        })?;
        let provenance = BackupProvenance {
            source_path: source_path
                .to_str()
                .ok_or_else(|| {
                    RecoveryStoreError::InvalidInput(
                        "backup source path must be valid UTF-8".to_owned(),
                    )
                })?
                .to_owned(),
            deployment_id: authority.deployment_id,
            instance_id: authority.instance_id,
            authority_generation: authority.authority_generation,
        };
        let created = create_private(destination)?;
        let result = (|| {
            self.conn
                .execute_batch("PRAGMA wal_checkpoint(FULL);")
                .map_err(super::map_sql_error)?;
            self.conn
                .backup(rusqlite::MAIN_DB, destination, None)
                .map_err(super::map_sql_error)?;
            created
                .sync_all()
                .map_err(|error| RecoveryStoreError::Io(format!("backup fsync failed: {error}")))?;
            write_provenance(destination, &provenance)
        })();
        if result.is_err() {
            drop(created);
            let _ = fs::remove_file(destination);
            let _ = fs::remove_file(provenance_path(destination));
        }
        result
    }

    /// Restores a database into a new path and immediately establishes a
    /// fresh authority namespace. The source is never modified and the
    /// destination must not exist before the operation.
    pub fn restore_rekey(
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        deployment_id: &str,
        instance_id: &str,
        release: RecoveryReleaseSet,
        now_millis: u64,
    ) -> Result<(Self, RecoveryBootContext), RecoveryStoreError> {
        let source = source.as_ref();
        let destination = destination.as_ref();
        super::support::reject_symlink_path(source)?;
        super::support::reject_symlink_path(destination)?;
        if !source.exists() {
            return Err(RecoveryStoreError::Io(
                "recovery backup source does not exist".to_owned(),
            ));
        }
        if source == destination || destination.exists() {
            return Err(RecoveryStoreError::BackupExists);
        }
        validate_uuid("deployment_id", deployment_id)?;
        validate_uuid("instance_id", instance_id)?;
        validate_wire(now_millis, "now_millis")?;
        release.validate()?;
        ensure_backup_parent(destination)?;
        let provenance = read_provenance(source)?;
        if provenance.instance_id != instance_id {
            return Err(RecoveryStoreError::ContractMismatch(
                "backup provenance identity does not match the restore namespace".to_owned(),
            ));
        }
        let live_path = PathBuf::from(&provenance.source_path);
        super::support::reject_symlink_path(&live_path)?;
        if !live_path.exists() {
            return Err(RecoveryStoreError::Io(
                "live authority high-water source is unavailable".to_owned(),
            ));
        }
        let live_lock_path = live_path.with_extension("gateway-recovery.lock");
        super::support::reject_symlink_path(&live_lock_path)?;
        let live_lock = super::support::open_private(&live_lock_path)?;
        live_lock
            .try_lock_exclusive()
            .map_err(|_| RecoveryStoreError::Busy)?;
        let live_conn = Connection::open(&live_path).map_err(super::map_sql_error)?;
        let live_generation: u64 = live_conn
            .query_row(
                "SELECT authority_generation FROM authority WHERE singleton = 1",
                [],
                |row| super::row_u64(row, 0),
            )
            .map_err(super::map_sql_error)?;
        if live_generation != provenance.authority_generation {
            return Err(RecoveryStoreError::ContractMismatch(
                "backup is older than the live authority high-water mark".to_owned(),
            ));
        }
        let source_conn = Connection::open(source).map_err(super::map_sql_error)?;
        let source_integrity: String = source_conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(super::map_sql_error)?;
        if source_integrity != "ok" {
            return Err(RecoveryStoreError::Corrupt(
                "recovery backup integrity check failed".to_owned(),
            ));
        }
        let created = create_private(destination)?;
        let backup_result = source_conn
            .backup(rusqlite::MAIN_DB, destination, None)
            .map_err(super::map_sql_error);
        drop(source_conn);
        if let Err(error) = backup_result {
            drop(created);
            cleanup_failed_restore(destination);
            return Err(error);
        }
        let sync_result = created.sync_all().map_err(|error| {
            RecoveryStoreError::Io(format!("restored backup fsync failed: {error}"))
        });
        drop(created);
        if let Err(error) = sync_result {
            cleanup_failed_restore(destination);
            return Err(error);
        }
        let result = (|| {
            let mut store = Self::open(destination)?;
            let boot = store.rekey_namespace(deployment_id, instance_id, release, now_millis)?;
            Ok((store, boot))
        })();
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                cleanup_failed_restore(destination);
                Err(error)
            }
        }
    }
}

fn ensure_backup_parent(path: &Path) -> Result<(), RecoveryStoreError> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            RecoveryStoreError::Io(format!("backup directory creation failed: {error}"))
        })?;
    }
    Ok(())
}

fn provenance_path(path: &Path) -> PathBuf {
    path.with_extension(PROVENANCE_SUFFIX)
}

fn write_provenance(
    destination: &Path,
    provenance: &BackupProvenance,
) -> Result<(), RecoveryStoreError> {
    let path = provenance_path(destination);
    let mut file = create_private(&path)?;
    let bytes = serde_json::to_vec(provenance).map_err(|error| {
        RecoveryStoreError::Io(format!("backup provenance encoding failed: {error}"))
    })?;
    file.write_all(&bytes).map_err(|error| {
        RecoveryStoreError::Io(format!("backup provenance write failed: {error}"))
    })?;
    file.sync_all()
        .map_err(|error| RecoveryStoreError::Io(format!("backup provenance fsync failed: {error}")))
}

fn read_provenance(source: &Path) -> Result<BackupProvenance, RecoveryStoreError> {
    let path = provenance_path(source);
    let bytes = fs::read(&path).map_err(|error| {
        RecoveryStoreError::Io(format!("backup provenance read failed: {error}"))
    })?;
    let provenance: BackupProvenance = serde_json::from_slice(&bytes).map_err(|error| {
        RecoveryStoreError::Corrupt(format!("backup provenance is invalid: {error}"))
    })?;
    if provenance.source_path.is_empty() || provenance.authority_generation == 0 {
        return Err(RecoveryStoreError::Corrupt(
            "backup provenance is incomplete".to_owned(),
        ));
    }
    Ok(provenance)
}

fn cleanup_failed_restore(destination: &Path) {
    let _ = fs::remove_file(destination);
    let _ = fs::remove_file(destination.with_extension("gateway-recovery.lock"));
    let _ = fs::remove_file(destination.with_extension("db-wal"));
    let _ = fs::remove_file(destination.with_extension("db-shm"));
    let _ = fs::remove_file(provenance_path(destination));
}

fn create_private(path: &Path) -> Result<File, RecoveryStoreError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| {
        RecoveryStoreError::Io(format!("backup destination creation failed: {error}"))
    })
}
