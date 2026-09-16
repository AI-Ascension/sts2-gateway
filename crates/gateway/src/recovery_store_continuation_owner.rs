// SPDX-License-Identifier: MIT

use rusqlite::{Connection, OptionalExtension, Transaction};

use super::super::recovery_types::{
    RecoveryContinuationOwner, RecoveryContinuationOwnerClaim,
    RecoveryContinuationOwnerClaimResult, RecoveryContinuationOwnerSnapshot,
    RecoveryContinuationOwnerState, RecoveryStoreError, validate_identity, validate_uuid,
    validate_uuid_v4, validate_wire,
};
use super::GatewayRecoveryStore;
use super::support::{map_sql_error, row_u64};
use crate::sha256_hex;

#[path = "recovery_store_continuation_owner_snapshot.rs"]
mod snapshot;

use self::snapshot::current_owner_snapshot;

const MAX_RETAINED_CONTINUATION_CLAIMS: u64 = 4_096;

impl GatewayRecoveryStore {
    /// Reads a retained claim record by identity. This history is not evidence
    /// that the owner is currently live; callers must use
    /// [`Self::current_continuation_owner`] independently.
    pub fn lookup_continuation_owner_claim(
        &self,
        operation_id: &str,
    ) -> Result<Option<RecoveryContinuationOwnerClaim>, RecoveryStoreError> {
        validate_uuid_v4("operation_id", operation_id)?;
        select_claim(&self.conn, operation_id)
    }

    /// Reads the current destination owner without returning the lease token.
    ///
    /// A persisted lease alone is not proof that this process can still act for
    /// it. The runtime caller must additionally compare this snapshot with its
    /// in-memory lease and host-installation acknowledgment before advertising
    /// `Available`.
    pub fn current_continuation_owner(
        &self,
        session_id: &str,
        now_millis: u64,
    ) -> Result<RecoveryContinuationOwnerSnapshot, RecoveryStoreError> {
        validate_identity("session_id", session_id)?;
        validate_wire(now_millis, "now_millis")?;
        current_owner_snapshot(&self.conn, session_id, now_millis)
    }

    /// Durably claims this exact live owner fence for one continuation operation.
    /// Repeating the same operation and owner is idempotent; an operation ID or
    /// lease already bound to another claim conflicts.
    pub fn claim_continuation_owner(
        &mut self,
        operation_id: &str,
        expected_owner: &RecoveryContinuationOwner,
        now_millis: u64,
    ) -> Result<RecoveryContinuationOwnerClaimResult, RecoveryStoreError> {
        validate_uuid_v4("operation_id", operation_id)?;
        validate_owner(expected_owner)?;
        validate_wire(now_millis, "now_millis")?;

        let request_digest = claim_digest(operation_id, expected_owner)?;
        let tx = self.transaction()?;
        let snapshot = current_owner_snapshot(&tx, &expected_owner.session_id, now_millis)?;
        if snapshot.state != RecoveryContinuationOwnerState::Available {
            return Err(match snapshot.state {
                RecoveryContinuationOwnerState::Expired => RecoveryStoreError::LeaseExpired,
                RecoveryContinuationOwnerState::Revoked => RecoveryStoreError::LeaseRevoked,
                RecoveryContinuationOwnerState::Absent => RecoveryStoreError::LeaseNotFound,
                RecoveryContinuationOwnerState::Unknown
                | RecoveryContinuationOwnerState::Available => RecoveryStoreError::StaleLease,
            });
        }
        if snapshot.owner.as_ref() != Some(expected_owner) {
            return Err(RecoveryStoreError::StaleLease);
        }

        let existing = select_claim(&tx, operation_id)?;
        if let Some(existing) = existing {
            if existing.request_digest == request_digest && existing.owner == *expected_owner {
                tx.commit().map_err(map_sql_error)?;
                return Ok(RecoveryContinuationOwnerClaimResult::Duplicate(existing));
            }
            return Err(RecoveryStoreError::OperationConflict);
        }

        let owner_claim: Option<String> = tx
            .query_row(
                "SELECT operation_id FROM continuation_owner_claims
                 WHERE lease_id = ?1 AND lease_epoch = ?2",
                rusqlite::params![expected_owner.lease_id, expected_owner.lease_epoch as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(map_sql_error)?;
        if owner_claim.is_some() {
            return Err(RecoveryStoreError::OperationConflict);
        }

        let retained: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM continuation_owner_claims",
                [],
                |row| row_u64(row, 0),
            )
            .map_err(map_sql_error)?;
        if retained >= MAX_RETAINED_CONTINUATION_CLAIMS {
            return Err(RecoveryStoreError::CapacityExceeded);
        }

        let claim = RecoveryContinuationOwnerClaim {
            operation_id: operation_id.to_owned(),
            request_digest,
            owner: expected_owner.clone(),
            claimed_at_millis: now_millis,
        };
        insert_claim(&tx, &claim)?;
        tx.commit().map_err(map_sql_error)?;
        Ok(RecoveryContinuationOwnerClaimResult::Created(claim))
    }
}

fn validate_owner(owner: &RecoveryContinuationOwner) -> Result<(), RecoveryStoreError> {
    validate_uuid("deployment_id", &owner.deployment_id)?;
    validate_uuid("instance_id", &owner.instance_id)?;
    validate_uuid_v4("instance_incarnation", &owner.instance_incarnation)?;
    validate_uuid_v4("boot_id", &owner.boot_id)?;
    validate_uuid_v4("host_fence_id", &owner.host_fence_id)?;
    validate_uuid_v4("lease_id", &owner.lease_id)?;
    validate_identity("session_id", &owner.session_id)?;
    validate_wire(owner.authority_generation, "authority_generation")?;
    validate_wire(owner.host_fence_generation, "host_fence_generation")?;
    validate_wire(owner.lease_epoch, "lease_epoch")?;
    validate_wire(owner.lease_expires_at_millis, "lease_expires_at_millis")?;
    Ok(())
}

fn claim_digest(
    operation_id: &str,
    owner: &RecoveryContinuationOwner,
) -> Result<String, RecoveryStoreError> {
    let bytes = serde_json::to_vec(&(operation_id, owner)).map_err(|_| {
        RecoveryStoreError::InvalidInput("continuation owner claim is not serializable".to_owned())
    })?;
    Ok(sha256_hex(&bytes))
}

fn select_claim(
    tx: &Connection,
    operation_id: &str,
) -> Result<Option<RecoveryContinuationOwnerClaim>, RecoveryStoreError> {
    tx.query_row(
        "SELECT request_digest, deployment_id, instance_id, instance_incarnation, boot_id,
                authority_generation, host_fence_id, host_fence_generation, lease_id,
                lease_epoch, session_id, lease_expires_at_millis, claimed_at_millis
         FROM continuation_owner_claims WHERE operation_id = ?1",
        [operation_id],
        |row| {
            Ok(RecoveryContinuationOwnerClaim {
                operation_id: operation_id.to_owned(),
                request_digest: row.get(0)?,
                owner: RecoveryContinuationOwner {
                    deployment_id: row.get(1)?,
                    instance_id: row.get(2)?,
                    instance_incarnation: row.get(3)?,
                    boot_id: row.get(4)?,
                    authority_generation: row_u64(row, 5)?,
                    host_fence_id: row.get(6)?,
                    host_fence_generation: row_u64(row, 7)?,
                    lease_id: row.get(8)?,
                    lease_epoch: row_u64(row, 9)?,
                    session_id: row.get(10)?,
                    lease_expires_at_millis: row_u64(row, 11)?,
                },
                claimed_at_millis: row_u64(row, 12)?,
            })
        },
    )
    .optional()
    .map_err(map_sql_error)
}

fn insert_claim(
    tx: &Transaction<'_>,
    claim: &RecoveryContinuationOwnerClaim,
) -> Result<(), RecoveryStoreError> {
    let owner = &claim.owner;
    let changed = tx
        .execute(
            "INSERT INTO continuation_owner_claims (
                operation_id, request_digest, deployment_id, instance_id,
                instance_incarnation, boot_id, authority_generation, host_fence_id,
                host_fence_generation, lease_id, lease_epoch, session_id,
                lease_expires_at_millis, claimed_at_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                claim.operation_id,
                claim.request_digest,
                owner.deployment_id,
                owner.instance_id,
                owner.instance_incarnation,
                owner.boot_id,
                owner.authority_generation as i64,
                owner.host_fence_id,
                owner.host_fence_generation as i64,
                owner.lease_id,
                owner.lease_epoch as i64,
                owner.session_id,
                owner.lease_expires_at_millis as i64,
                claim.claimed_at_millis as i64,
            ],
        )
        .map_err(map_sql_error)?;
    if changed != 1 {
        return Err(RecoveryStoreError::PersistenceUnavailable);
    }
    Ok(())
}
