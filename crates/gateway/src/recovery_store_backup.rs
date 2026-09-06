// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::path::Path;

use rusqlite::Connection;

use super::super::recovery_types::{
    MAX_WIRE_INTEGER, RecoveryBootContext, RecoveryBootState, RecoveryReleaseSet,
    RecoveryStoreError, RecoveryUncertaintyReason, validate_uuid, validate_wire,
};
use super::super::{GatewayRecoveryStore, random_uuid};

impl GatewayRecoveryStore {
    /// Produces an online SQLite backup without replacing an existing path.
    /// The owner lock remains held for the lifetime of this store, and the
    /// destination is fsynced before this method returns.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), RecoveryStoreError> {
        let destination = destination.as_ref();
        if destination == self.path() || destination.exists() {
            return Err(RecoveryStoreError::BackupExists);
        }
        ensure_backup_parent(destination)?;
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
                .map_err(|error| RecoveryStoreError::Io(format!("backup fsync failed: {error}")))
        })();
        if result.is_err() {
            drop(created);
            let _ = fs::remove_file(destination);
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
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        created.sync_all().map_err(|error| {
            RecoveryStoreError::Io(format!("restored backup fsync failed: {error}"))
        })?;
        drop(created);
        let mut store = Self::open(destination)?;
        let boot = store.rekey_namespace(deployment_id, instance_id, release, now_millis)?;
        Ok((store, boot))
    }

    /// Atomically replaces the deployment namespace on restored state and
    /// starts a fresh boot. Historical operation rows keep their original
    /// deployment context and therefore cannot authorize a mutation.
    pub fn rekey_namespace(
        &mut self,
        deployment_id: &str,
        instance_id: &str,
        release: RecoveryReleaseSet,
        now_millis: u64,
    ) -> Result<RecoveryBootContext, RecoveryStoreError> {
        validate_uuid("deployment_id", deployment_id)?;
        validate_uuid("instance_id", instance_id)?;
        validate_wire(now_millis, "now_millis")?;
        release.validate()?;
        let current = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if current.instance_id != instance_id {
            return Err(RecoveryStoreError::ContractMismatch(
                "rekey instance identity differs from the restored store".to_owned(),
            ));
        }
        if current.release != release {
            return Err(RecoveryStoreError::ReleaseMismatch);
        }
        let generation = current
            .authority_generation
            .checked_add(1)
            .filter(|value| *value <= MAX_WIRE_INTEGER)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        let boot_id = random_uuid();
        let incarnation = random_uuid();
        let tx = self.transaction()?;
        tx.execute(
            "UPDATE leases SET status = 'REVOKED', revoked_reason = 'rekey'
             WHERE status = 'ACTIVE'",
            [],
        )
        .map_err(super::map_sql_error)?;
        tx.execute(
            "UPDATE operations SET state = 'UNKNOWN', uncertainty_reason = ?1,
                    updated_at_millis = ?2
             WHERE state IN ('INTENT_RECORDED', 'MAY_HAVE_BEEN_DISPATCHED', 'ACCEPTED', 'UNKNOWN')",
            rusqlite::params![
                RecoveryUncertaintyReason::AuthorityRotated.as_str(),
                now_millis as i64,
            ],
        )
        .map_err(super::map_sql_error)?;
        tx.execute(
            "UPDATE authority SET deployment_id = ?1, instance_id = ?2,
                    instance_incarnation = ?3, boot_id = ?4,
                    authority_generation = ?5, release_digest = ?6,
                    config_digest = ?7, profile_digest = ?8,
                    runtime_v3_schema_digest = ?9, created_at_millis = ?10,
                    state = ?11, host_fence_id = NULL,
                    fence_generation = NULL, host_fence_at = NULL
             WHERE singleton = 1",
            rusqlite::params![
                deployment_id,
                instance_id,
                incarnation,
                boot_id,
                generation as i64,
                release.release_digest,
                release.config_digest,
                release.profile_digest,
                release.runtime_v3_schema_digest,
                now_millis as i64,
                RecoveryBootState::FenceRequired.as_str(),
            ],
        )
        .map_err(super::map_sql_error)?;
        tx.commit().map_err(super::map_sql_error)?;
        Ok(RecoveryBootContext {
            deployment_id: deployment_id.to_owned(),
            instance_id: instance_id.to_owned(),
            instance_incarnation: incarnation,
            boot_id,
            authority_generation: generation,
            release,
            created_at_millis: now_millis,
            state: RecoveryBootState::FenceRequired,
        })
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
