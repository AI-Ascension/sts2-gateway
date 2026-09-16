// SPDX-License-Identifier: MIT

use rusqlite::{Connection, OptionalExtension};

use super::super::super::recovery_types::{
    RecoveryBootState, RecoveryContinuationOwner, RecoveryContinuationOwnerSnapshot,
    RecoveryContinuationOwnerState, RecoveryHostLeaseState, RecoveryLeaseState, RecoveryStoreError,
};
use super::super::support::{map_sql_error, row_u64};

pub(super) fn current_owner_snapshot(
    conn: &Connection,
    session_id: &str,
    now_millis: u64,
) -> Result<RecoveryContinuationOwnerSnapshot, RecoveryStoreError> {
    let Some(authority) = conn
        .query_row(
            "SELECT deployment_id, instance_id, instance_incarnation, boot_id,
                    authority_generation, release_digest, config_digest, profile_digest,
                    runtime_v3_schema_digest, created_at_millis, state
             FROM authority WHERE singleton = 1",
            [],
            super::super::support::row_boot,
        )
        .optional()
        .map_err(map_sql_error)?
    else {
        return Ok(snapshot(RecoveryContinuationOwnerState::Absent, None));
    };
    if authority.state != RecoveryBootState::Ready {
        return Ok(snapshot(RecoveryContinuationOwnerState::Unknown, None));
    }
    let Some(fence) = conn
        .query_row(
            "SELECT host_fence_id, deployment_id, instance_id, instance_incarnation,
                    boot_id, authority_generation, fence_generation, host_fence_at
             FROM authority WHERE singleton = 1 AND host_fence_id IS NOT NULL",
            [],
            super::super::support::row_fence,
        )
        .optional()
        .map_err(map_sql_error)?
    else {
        return Ok(snapshot(RecoveryContinuationOwnerState::Unknown, None));
    };
    if fence.deployment_id != authority.deployment_id
        || fence.instance_id != authority.instance_id
        || fence.instance_incarnation != authority.instance_incarnation
        || fence.boot_id != authority.boot_id
        || fence.authority_generation != authority.authority_generation
    {
        return Ok(snapshot(RecoveryContinuationOwnerState::Unknown, None));
    }

    let lease = conn
        .query_row(
            "SELECT lease_id, lease_epoch, session_id, expires_at_millis, status,
                    host_fence_id, host_fence_generation, host_state
             FROM leases
             WHERE deployment_id = ?1 AND instance_id = ?2
               AND instance_incarnation = ?3 AND boot_id = ?4
               AND authority_generation = ?5
             ORDER BY lease_epoch DESC LIMIT 1",
            rusqlite::params![
                authority.deployment_id,
                authority.instance_id,
                authority.instance_incarnation,
                authority.boot_id,
                authority.authority_generation as i64,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row_u64(row, 1)?,
                    row.get::<_, String>(2)?,
                    row_u64(row, 3)?,
                    super::super::support::parse_lease_state(row.get(4)?)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    RecoveryHostLeaseState::parse(&row.get::<_, String>(7)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_error)?;
    let Some((
        lease_id,
        lease_epoch,
        stored_session_id,
        lease_expires_at_millis,
        lease_state,
        host_fence_id,
        host_fence_generation,
        host_state,
    )) = lease
    else {
        return Ok(snapshot(RecoveryContinuationOwnerState::Absent, None));
    };
    let Some(host_fence_generation) =
        host_fence_generation.and_then(|value| u64::try_from(value).ok())
    else {
        return Ok(snapshot(RecoveryContinuationOwnerState::Unknown, None));
    };
    let owner = RecoveryContinuationOwner {
        deployment_id: authority.deployment_id,
        instance_id: authority.instance_id,
        instance_incarnation: authority.instance_incarnation,
        boot_id: authority.boot_id,
        authority_generation: authority.authority_generation,
        host_fence_id: fence.host_fence_id.clone(),
        host_fence_generation: fence.fence_generation,
        lease_id,
        lease_epoch,
        session_id: stored_session_id,
        lease_expires_at_millis,
    };
    if host_fence_id.as_deref() != Some(owner.host_fence_id.as_str())
        || host_fence_generation != owner.host_fence_generation
        || owner.session_id != session_id
    {
        return Ok(snapshot(
            RecoveryContinuationOwnerState::Unknown,
            Some(owner),
        ));
    }
    let state = match lease_state {
        RecoveryLeaseState::Active if owner.lease_expires_at_millis > now_millis => {
            if host_state == RecoveryHostLeaseState::Installed {
                RecoveryContinuationOwnerState::Available
            } else {
                RecoveryContinuationOwnerState::Unknown
            }
        }
        RecoveryLeaseState::Active | RecoveryLeaseState::Expired => {
            RecoveryContinuationOwnerState::Expired
        }
        RecoveryLeaseState::Revoked => RecoveryContinuationOwnerState::Revoked,
    };
    Ok(snapshot(state, Some(owner)))
}

pub(super) fn snapshot(
    state: RecoveryContinuationOwnerState,
    owner: Option<RecoveryContinuationOwner>,
) -> RecoveryContinuationOwnerSnapshot {
    RecoveryContinuationOwnerSnapshot { state, owner }
}
