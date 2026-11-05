// SPDX-License-Identifier: MIT

//! Pre-adoption prototype: durable claims for a finite preapproved native-root pool.
//! This is test-only until ADR0039 has owner and native-host acceptance. It never
//! resolves host paths, launches processes, selects game saves or authorizes idle.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use super::preapproved_root_pool_decode::positive_persisted_u64;
use super::types::UserDataIdentity;
use crate::process_profile::LaunchProfileId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApprovedRoot {
    pub key: String,
    pub profile: LaunchProfileId,
    pub namespace: u64,
    pub image_digest: String,
}

#[derive(Clone, Debug)]
pub(super) struct OwnerExclusion {
    pub root: ApprovedRoot,
    // An affirmative owner statement is scoped to the requesting instance and lease.
    pub instance: String,
    pub lease_epoch: u64,
    pub owner_epoch: u64,
    pub process_generation: u64,
    pub no_retained_owner: bool,
    pub no_unsettled_operation: bool,
    pub no_foreign_marker: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ClaimRequest {
    pub identity: UserDataIdentity,
    pub instance: String,
    pub operation: String,
    pub lease_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RootClaim {
    pub root: ApprovedRoot,
    pub identity: UserDataIdentity,
    pub instance: String,
    pub operation: String,
    pub lease_epoch: u64,
    pub owner_epoch: u64,
    pub process_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PoolError {
    InvalidCatalog,
    UnverifiedOwner,
    Conflict,
    Storage,
}

/// Source-only allocator. Its rows cannot be released or reassigned; any uncertain
/// outcome remains retained. Production composition deliberately has no constructor.
pub(super) struct CandidateRootPool {
    connection: Connection,
    catalog: BTreeMap<String, ApprovedRoot>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sql_error(_: rusqlite::Error) -> PoolError {
    PoolError::Storage
}

fn load_claim(
    connection: &Connection,
    identity: UserDataIdentity,
) -> Result<Option<RootClaim>, PoolError> {
    connection
        .query_row(
            "SELECT root_key, native_profile, namespace, image_digest, identity,
                    instance, operation, lease_epoch, owner_epoch, process_generation
             FROM root_claims WHERE identity=?1",
            params![i64::try_from(identity.value()).map_err(|_| PoolError::InvalidCatalog)?],
            |row| {
                Ok(RootClaim {
                    root: ApprovedRoot {
                        key: row.get(0)?,
                        profile: LaunchProfileId::new(positive_persisted_u64(row, 1)?),
                        namespace: positive_persisted_u64(row, 2)?,
                        image_digest: row.get(3)?,
                    },
                    identity: UserDataIdentity::new(positive_persisted_u64(row, 4)?),
                    instance: row.get(5)?,
                    operation: row.get(6)?,
                    lease_epoch: positive_persisted_u64(row, 7)?,
                    owner_epoch: positive_persisted_u64(row, 8)?,
                    process_generation: positive_persisted_u64(row, 9)?,
                })
            },
        )
        .optional()
        .map_err(sql_error)
}

impl CandidateRootPool {
    /// The caller must supply an operator-approved, already-validated native
    /// catalog; this prototype intentionally cannot read deployment config itself.
    pub(super) fn open(path: &Path, roots: Vec<ApprovedRoot>) -> Result<Self, PoolError> {
        if !path.is_absolute() || roots.is_empty() || roots.len() > 64 {
            return Err(PoolError::InvalidCatalog);
        }
        let mut catalog = BTreeMap::new();
        let mut profiles = BTreeSet::new();
        let mut namespaces = BTreeSet::new();
        for root in roots {
            if !valid_id(&root.key)
                || root.profile.value() == 0
                || root.namespace == 0
                || !valid_digest(&root.image_digest)
                || !profiles.insert(root.profile.value())
                || !namespaces.insert(root.namespace)
                || catalog.insert(root.key.clone(), root).is_some()
            {
                return Err(PoolError::InvalidCatalog);
            }
        }
        let connection = Connection::open(path).map_err(sql_error)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(sql_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS root_claims (
                 root_key TEXT PRIMARY KEY NOT NULL,
                 native_profile INTEGER NOT NULL,
                 namespace INTEGER NOT NULL,
                 image_digest TEXT NOT NULL,
                 identity INTEGER NOT NULL UNIQUE,
                 instance TEXT NOT NULL,
                 operation TEXT NOT NULL,
                 lease_epoch INTEGER NOT NULL,
                 owner_epoch INTEGER NOT NULL,
                 process_generation INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS root_claims_instance_operation
               ON root_claims(instance, operation);",
            )
            .map_err(sql_error)?;
        {
            let mut query = connection
                .prepare(
                    "SELECT root_key, native_profile, namespace, image_digest,
                        identity, lease_epoch, owner_epoch, process_generation
                 FROM root_claims",
                )
                .map_err(sql_error)?;
            let mut rows = query.query([]).map_err(sql_error)?;
            while let Some(row) = rows.next().map_err(sql_error)? {
                let key: String = row.get(0).map_err(sql_error)?;
                let root = catalog.get(&key).ok_or(PoolError::InvalidCatalog)?;
                let profile: i64 = row.get(1).map_err(sql_error)?;
                let namespace: i64 = row.get(2).map_err(sql_error)?;
                let digest: String = row.get(3).map_err(sql_error)?;
                let identity: i64 = row.get(4).map_err(sql_error)?;
                let lease_epoch: i64 = row.get(5).map_err(sql_error)?;
                let owner_epoch: i64 = row.get(6).map_err(sql_error)?;
                let generation: i64 = row.get(7).map_err(sql_error)?;
                if profile <= 0
                    || profile as u64 != root.profile.value()
                    || namespace <= 0
                    || namespace as u64 != root.namespace
                    || digest != root.image_digest
                    || identity <= 0
                    || lease_epoch <= 0
                    || owner_epoch <= 0
                    || generation <= 0
                {
                    return Err(PoolError::InvalidCatalog);
                }
            }
        }
        Ok(Self {
            connection,
            catalog,
        })
    }

    /// Recovers an exact durable claim after a lost response or restart.
    /// This lookup grants no native-process, host-save, or negative-idle authority
    /// and never reserves, releases, or reassigns a root.
    pub(super) fn reconcile(&self, request: &ClaimRequest) -> Result<Option<RootClaim>, PoolError> {
        if request.identity.value() == 0
            || request.lease_epoch == 0
            || !valid_id(&request.instance)
            || !valid_id(&request.operation)
        {
            return Err(PoolError::InvalidCatalog);
        }
        match load_claim(&self.connection, request.identity)? {
            None => Ok(None),
            Some(existing) => {
                if self.catalog.get(&existing.root.key) != Some(&existing.root) {
                    return Err(PoolError::InvalidCatalog);
                }
                if existing.instance != request.instance
                    || existing.operation != request.operation
                    || existing.lease_epoch != request.lease_epoch
                {
                    return Err(PoolError::Conflict);
                }
                Ok(Some(existing))
            }
        }
    }

    /// Only an affirmative, exact owner witness may reserve a candidate.
    /// This is not a real owner oracle and cannot be used for production admission.
    pub(super) fn reserve(
        &mut self,
        request: ClaimRequest,
        witness: OwnerExclusion,
    ) -> Result<RootClaim, PoolError> {
        let approved = self
            .catalog
            .get(&witness.root.key)
            .ok_or(PoolError::InvalidCatalog)?;
        if witness.root != *approved
            || !witness.no_retained_owner
            || !witness.no_unsettled_operation
            || !witness.no_foreign_marker
            || witness.instance != request.instance
            || witness.lease_epoch != request.lease_epoch
            || witness.owner_epoch == 0
            || witness.process_generation == 0
        {
            return Err(PoolError::UnverifiedOwner);
        }
        if request.identity.value() == 0
            || request.lease_epoch == 0
            || !valid_id(&request.instance)
            || !valid_id(&request.operation)
        {
            return Err(PoolError::InvalidCatalog);
        }
        let claim = RootClaim {
            root: approved.clone(),
            identity: request.identity,
            instance: request.instance,
            operation: request.operation,
            lease_epoch: request.lease_epoch,
            owner_epoch: witness.owner_epoch,
            process_generation: witness.process_generation,
        };
        let invalid = |_| PoolError::InvalidCatalog;
        let identity = i64::try_from(claim.identity.value()).map_err(invalid)?;
        let profile = i64::try_from(claim.root.profile.value()).map_err(invalid)?;
        let namespace = i64::try_from(claim.root.namespace).map_err(invalid)?;
        let lease_epoch = i64::try_from(claim.lease_epoch).map_err(invalid)?;
        let owner_epoch = i64::try_from(claim.owner_epoch).map_err(invalid)?;
        let generation = i64::try_from(claim.process_generation).map_err(invalid)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        if let Some(existing) = load_claim(&transaction, claim.identity)? {
            return if existing == claim {
                Ok(existing)
            } else {
                Err(PoolError::Conflict)
            };
        }
        transaction
            .execute(
                "INSERT INTO root_claims (
                root_key, native_profile, namespace, image_digest, identity,
                instance, operation, lease_epoch, owner_epoch, process_generation
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    &claim.root.key,
                    profile,
                    namespace,
                    &claim.root.image_digest,
                    identity,
                    &claim.instance,
                    &claim.operation,
                    lease_epoch,
                    owner_epoch,
                    generation
                ],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.code == ErrorCode::ConstraintViolation =>
                {
                    PoolError::Conflict
                }
                _ => PoolError::Storage,
            })?;
        transaction.commit().map_err(sql_error)?;
        Ok(claim)
    }
}
