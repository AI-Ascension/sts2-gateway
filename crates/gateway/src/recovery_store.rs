// SPDX-License-Identifier: MIT

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, Transaction};

use super::recovery_types::{
    RecoveryBootContext, RecoveryBootState, RecoveryHostFence, RecoveryLease, RecoveryLeaseProof,
    RecoveryLeaseState, RecoveryStoreConfig, RecoveryStoreError, validate_token, validate_uuid,
    validate_uuid_v4,
};
use super::sha256_hex;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub struct GatewayRecoveryStore {
    pub(super) conn: Connection,
    pub(super) _lock: File,
    path: PathBuf,
    pub(super) config: RecoveryStoreConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryStorePragmas {
    pub journal_mode: String,
    pub synchronous: i64,
    pub foreign_keys: bool,
    pub busy_timeout_millis: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryLeaseRequest {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub host_fence_id: String,
    pub host_fence_generation: u64,
    pub caller_id: String,
    pub session_id: String,
    pub now_millis: u64,
    pub ttl_seconds: u64,
    pub renewal_interval_seconds: u64,
}

#[path = "recovery_store_archive.rs"]
mod archive;
#[path = "recovery_store_authority.rs"]
mod authority;
#[path = "recovery_store_backup.rs"]
mod backup;
#[path = "recovery_store_lease.rs"]
mod lease;
#[path = "recovery_store_operation_helpers.rs"]
mod operation_helpers;
#[path = "recovery_store_operation_persistence.rs"]
mod operation_persistence;
#[path = "recovery_store_operations.rs"]
mod operations;
#[path = "recovery_store_sql.rs"]
mod sql;
#[path = "recovery_store_support.rs"]
mod support;
#[path = "recovery_store_tickets.rs"]
mod tickets;

use support::{
    ensure_parent, migrate, open_private, parse_lease_state, row_boot, row_fence, row_lease,
};
pub(super) use support::{map_sql_error, row_u64};

impl GatewayRecoveryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RecoveryStoreError> {
        Self::open_with_config(path, RecoveryStoreConfig::default())
    }

    pub fn open_with_config(
        path: impl AsRef<Path>,
        config: RecoveryStoreConfig,
    ) -> Result<Self, RecoveryStoreError> {
        config.validate()?;
        let path = path.as_ref().to_path_buf();
        ensure_parent(&path)?;
        let lock_path = path.with_extension("gateway-recovery.lock");
        let lock = open_private(&lock_path)?;
        lock.try_lock_exclusive()
            .map_err(|_| RecoveryStoreError::Busy)?;
        let database = open_private(&path)?;
        drop(database);
        let conn = Connection::open(&path).map_err(map_sql_error)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(map_sql_error)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA wal_autocheckpoint = 1000;",
        )
        .map_err(map_sql_error)?;
        migrate(&conn)?;
        Ok(Self {
            conn,
            _lock: lock,
            path,
            config,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn pragmas(&self) -> Result<RecoveryStorePragmas, RecoveryStoreError> {
        let journal_mode = self
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(map_sql_error)?;
        let synchronous = self
            .conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .map_err(map_sql_error)?;
        let foreign_keys = self
            .conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .map_err(map_sql_error)?
            == 1;
        let busy_timeout_millis = self
            .conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(map_sql_error)?;
        Ok(RecoveryStorePragmas {
            journal_mode,
            synchronous,
            foreign_keys,
            busy_timeout_millis,
        })
    }

    pub fn integrity_check(&self) -> Result<(), RecoveryStoreError> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(map_sql_error)?;
        if result != "ok" {
            return Err(RecoveryStoreError::Corrupt(
                "recovery store integrity check failed".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64, RecoveryStoreError> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(map_sql_error)
    }

    pub(super) fn transaction<'a>(&'a mut self) -> Result<Transaction<'a>, RecoveryStoreError> {
        self.conn.transaction().map_err(map_sql_error)
    }

    pub(super) fn current_authority(
        &self,
    ) -> Result<Option<RecoveryBootContext>, RecoveryStoreError> {
        self.conn
            .query_row(
                "SELECT deployment_id, instance_id, instance_incarnation, boot_id,
                        authority_generation, release_digest, config_digest, profile_digest,
                        runtime_v3_schema_digest, created_at_millis, state
                 FROM authority WHERE singleton = 1",
                [],
                row_boot,
            )
            .optional()
            .map_err(map_sql_error)
    }

    pub(super) fn current_fence(&self) -> Result<Option<RecoveryHostFence>, RecoveryStoreError> {
        self.conn
            .query_row(
                "SELECT host_fence_id, deployment_id, instance_id, instance_incarnation,
                        boot_id, authority_generation, fence_generation, host_fence_at
                 FROM authority WHERE singleton = 1 AND host_fence_id IS NOT NULL",
                [],
                row_fence,
            )
            .optional()
            .map_err(map_sql_error)
    }

    pub(super) fn lease_by_id(
        &self,
        lease_id: &str,
    ) -> Result<Option<RecoveryLease>, RecoveryStoreError> {
        self.conn
            .query_row(
                "SELECT deployment_id, instance_id, instance_incarnation, boot_id,
                        authority_generation, lease_id, lease_epoch, fence_token_hash,
                        issued_at_millis, expires_at_millis, ttl_seconds,
                        renewal_interval_seconds, last_renew_sequence
                 FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| row_lease(row, ""),
            )
            .optional()
            .map_err(map_sql_error)
            .map(|lease| {
                lease.map(|mut value| {
                    value.fence_token.clear();
                    value
                })
            })
    }

    pub(super) fn active_lease(
        &self,
    ) -> Result<Option<(RecoveryLease, RecoveryLeaseState)>, RecoveryStoreError> {
        self.conn
            .query_row(
                "SELECT deployment_id, instance_id, instance_incarnation, boot_id,
                        authority_generation, lease_id, lease_epoch, fence_token_hash,
                        issued_at_millis, expires_at_millis, ttl_seconds,
                        renewal_interval_seconds, last_renew_sequence, status
                 FROM leases WHERE status = 'ACTIVE' ORDER BY lease_epoch DESC LIMIT 1",
                [],
                |row| {
                    let lease = row_lease(row, "")?;
                    let status = parse_lease_state(row.get(13)?)?;
                    Ok((lease, status))
                },
            )
            .optional()
            .map_err(map_sql_error)
    }

    pub(super) fn ensure_context(
        &self,
        proof: &RecoveryLeaseProof,
        now_millis: u64,
    ) -> Result<RecoveryLease, RecoveryStoreError> {
        validate_uuid("deployment_id", &proof.deployment_id)?;
        validate_uuid("instance_id", &proof.instance_id)?;
        validate_uuid_v4("instance_incarnation", &proof.instance_incarnation)?;
        validate_uuid_v4("boot_id", &proof.boot_id)?;
        validate_uuid_v4("lease_id", &proof.lease_id)?;
        validate_token("fence_token", &proof.fence_token)?;
        let Some(lease) = self.lease_by_id_with_token(&proof.lease_id, &proof.fence_token)? else {
            return Err(RecoveryStoreError::LeaseNotFound);
        };
        if lease.deployment_id != proof.deployment_id
            || lease.instance_id != proof.instance_id
            || lease.instance_incarnation != proof.instance_incarnation
            || lease.boot_id != proof.boot_id
            || lease.authority_generation != proof.authority_generation
            || lease.lease_epoch != proof.lease_epoch
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        if lease.expires_at_millis <= now_millis {
            return Err(RecoveryStoreError::LeaseExpired);
        }
        let Some(authority) = self.current_authority()? else {
            return Err(RecoveryStoreError::AuthorityNotFound);
        };
        if authority.state != RecoveryBootState::Ready
            || authority.boot_id != proof.boot_id
            || authority.instance_incarnation != proof.instance_incarnation
            || authority.authority_generation != proof.authority_generation
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(lease)
    }

    pub(super) fn lease_by_id_with_token(
        &self,
        lease_id: &str,
        token: &str,
    ) -> Result<Option<RecoveryLease>, RecoveryStoreError> {
        let Some(mut lease) = self.lease_by_id(lease_id)? else {
            return Ok(None);
        };
        if sha256_hex(token.as_bytes())
            != self
                .conn
                .query_row(
                    "SELECT fence_token_hash FROM leases WHERE lease_id = ?1",
                    [lease_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(map_sql_error)?
        {
            return Ok(None);
        }
        lease.fence_token = token.to_owned();
        Ok(Some(lease))
    }
}
