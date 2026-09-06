// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    MAX_WIRE_INTEGER, RecoveryBootContext, RecoveryBootState, RecoveryHostFence, RecoveryLease,
    RecoveryReleaseSet, RecoveryStoreError, RecoveryUncertaintyReason, validate_identity,
    validate_uuid, validate_uuid_v4, validate_wire,
};
use super::super::{random_token, random_uuid, sha256_hex};
use super::GatewayRecoveryStore;
use super::RecoveryLeaseRequest;

impl GatewayRecoveryStore {
    pub fn start_boot(
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
        let boot_id = random_uuid();
        let incarnation = random_uuid();
        let tx = self.transaction()?;
        let old = tx
            .query_row(
                "SELECT deployment_id, instance_id, authority_generation,
                        release_digest, config_digest, profile_digest, runtime_v3_schema_digest
                 FROM authority WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        super::row_u64(row, 2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(super::map_sql_error)?;
        let generation = match old {
            Some((old_deployment, old_instance, old_generation, a, b, c, d)) => {
                if old_deployment != deployment_id || old_instance != instance_id {
                    return Err(RecoveryStoreError::ContractMismatch(
                        "deployment or instance identity differs from the store".to_owned(),
                    ));
                }
                if (a, b, c, d)
                    != (
                        release.release_digest.clone(),
                        release.config_digest.clone(),
                        release.profile_digest.clone(),
                        release.runtime_v3_schema_digest.clone(),
                    )
                {
                    return Err(RecoveryStoreError::ReleaseMismatch);
                }
                old_generation
                    .checked_add(1)
                    .filter(|value| *value <= MAX_WIRE_INTEGER)
                    .ok_or(RecoveryStoreError::CounterExhausted)?
            }
            None => 1,
        };
        tx.execute(
            "UPDATE leases SET status = 'REVOKED', revoked_reason = 'incarnation_replaced'
             WHERE status = 'ACTIVE'",
            [],
        )
        .map_err(super::map_sql_error)?;
        tx.execute(
            "UPDATE operations SET state = 'UNKNOWN', uncertainty_reason = ?1,
                    updated_at_millis = ?2
             WHERE state IN ('INTENT_RECORDED', 'MAY_HAVE_BEEN_DISPATCHED', 'ACCEPTED', 'UNKNOWN')",
            (
                RecoveryUncertaintyReason::AuthorityRotated.as_str(),
                now_millis as i64,
            ),
        )
        .map_err(super::map_sql_error)?;
        tx.execute(
            "INSERT INTO authority (
                singleton, deployment_id, instance_id, instance_incarnation, boot_id,
                authority_generation, release_digest, config_digest, profile_digest,
                runtime_v3_schema_digest, created_at_millis, state,
                host_fence_id, fence_generation, host_fence_at
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL)
             ON CONFLICT(singleton) DO UPDATE SET
                deployment_id = excluded.deployment_id,
                instance_id = excluded.instance_id,
                instance_incarnation = excluded.instance_incarnation,
                boot_id = excluded.boot_id,
                authority_generation = excluded.authority_generation,
                release_digest = excluded.release_digest,
                config_digest = excluded.config_digest,
                profile_digest = excluded.profile_digest,
                runtime_v3_schema_digest = excluded.runtime_v3_schema_digest,
                created_at_millis = excluded.created_at_millis,
                state = excluded.state,
                host_fence_id = NULL,
                fence_generation = NULL,
                host_fence_at = NULL",
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

    pub fn complete_host_fence(
        &mut self,
        boot: &RecoveryBootContext,
        now_millis: u64,
    ) -> Result<RecoveryHostFence, RecoveryStoreError> {
        validate_wire(now_millis, "now_millis")?;
        let current = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if current != *boot || current.state != RecoveryBootState::FenceRequired {
            return Err(RecoveryStoreError::StaleLease);
        }
        let fence = RecoveryHostFence {
            host_fence_id: random_uuid(),
            deployment_id: boot.deployment_id.clone(),
            instance_id: boot.instance_id.clone(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            fence_generation: boot.authority_generation,
            created_at_millis: now_millis,
        };
        let changed = self
            .conn
            .execute(
                "UPDATE authority SET state = ?1, host_fence_id = ?2,
                        fence_generation = ?3, host_fence_at = ?4
                 WHERE singleton = 1 AND boot_id = ?5 AND authority_generation = ?6
                   AND state = ?7",
                rusqlite::params![
                    RecoveryBootState::Ready.as_str(),
                    fence.host_fence_id,
                    fence.fence_generation as i64,
                    now_millis as i64,
                    boot.boot_id,
                    boot.authority_generation as i64,
                    RecoveryBootState::FenceRequired.as_str(),
                ],
            )
            .map_err(super::map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(fence)
    }

    pub fn acquire_lease(
        &mut self,
        request: RecoveryLeaseRequest,
    ) -> Result<RecoveryLease, RecoveryStoreError> {
        validate_uuid("deployment_id", &request.deployment_id)?;
        validate_uuid("instance_id", &request.instance_id)?;
        validate_uuid_v4("instance_incarnation", &request.instance_incarnation)?;
        validate_uuid_v4("boot_id", &request.boot_id)?;
        validate_uuid_v4("host_fence_id", &request.host_fence_id)?;
        validate_wire(request.now_millis, "now_millis")?;
        validate_identity("caller_id", &request.caller_id)?;
        validate_identity("session_id", &request.session_id)?;
        validate_identity("host_fence_id", &request.host_fence_id)?;
        validate_wire(request.authority_generation, "authority_generation")?;
        validate_wire(request.host_fence_generation, "fence_generation")?;
        if !(5..=300).contains(&request.ttl_seconds)
            || request.renewal_interval_seconds == 0
            || request.renewal_interval_seconds >= request.ttl_seconds
        {
            return Err(RecoveryStoreError::InvalidInput(
                "lease policy must satisfy 1 <= renewal < ttl <= 300".to_owned(),
            ));
        }
        let authority = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if authority.state != RecoveryBootState::Ready
            || authority.deployment_id != request.deployment_id
            || authority.instance_id != request.instance_id
            || authority.instance_incarnation != request.instance_incarnation
            || authority.boot_id != request.boot_id
            || authority.authority_generation != request.authority_generation
        {
            return Err(RecoveryStoreError::HostFenceRequired);
        }
        let fence = self
            .current_fence()?
            .ok_or(RecoveryStoreError::HostFenceRequired)?;
        if fence.host_fence_id != request.host_fence_id
            || fence.fence_generation != request.host_fence_generation
            || fence.deployment_id != request.deployment_id
            || fence.instance_id != request.instance_id
            || fence.instance_incarnation != request.instance_incarnation
            || fence.boot_id != request.boot_id
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        self.expire_leases(request.now_millis)?;
        if self.active_lease()?.is_some() {
            return Err(RecoveryStoreError::Busy);
        }
        let next_epoch = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(lease_epoch), 0) + 1 FROM leases",
                [],
                |row| super::row_u64(row, 0),
            )
            .map_err(super::map_sql_error)?;
        validate_wire(next_epoch, "lease_epoch")?;
        let lease_id = random_uuid();
        let token = random_token();
        let ttl_millis = request
            .ttl_seconds
            .checked_mul(1_000)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        let expires = request
            .now_millis
            .checked_add(ttl_millis)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        self.conn
            .execute(
                "INSERT INTO leases (
                    lease_id, deployment_id, instance_id, instance_incarnation, boot_id,
                    authority_generation, lease_epoch, fence_token_hash, issued_at_millis,
                    expires_at_millis, ttl_seconds, renewal_interval_seconds,
                    last_renew_sequence, caller_id, session_id, status, revoked_reason
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?14, 'ACTIVE', NULL)",
                rusqlite::params![
                    lease_id,
                    request.deployment_id,
                    request.instance_id,
                    request.instance_incarnation,
                    request.boot_id,
                    request.authority_generation as i64,
                    next_epoch as i64,
                    sha256_hex(token.as_bytes()),
                    request.now_millis as i64,
                    expires as i64,
                    request.ttl_seconds as i64,
                    request.renewal_interval_seconds as i64,
                    request.caller_id,
                    request.session_id,
                ],
            )
            .map_err(super::map_sql_error)?;
        Ok(RecoveryLease {
            deployment_id: request.deployment_id,
            instance_id: request.instance_id,
            instance_incarnation: request.instance_incarnation,
            boot_id: request.boot_id,
            authority_generation: request.authority_generation,
            lease_id,
            lease_epoch: next_epoch,
            fence_token: token,
            issued_at_millis: request.now_millis,
            expires_at_millis: expires,
            ttl_seconds: request.ttl_seconds,
            renewal_interval_seconds: request.renewal_interval_seconds,
            last_renew_sequence: 0,
        })
    }

    pub fn current_boot(&self) -> Result<RecoveryBootContext, RecoveryStoreError> {
        self.current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)
    }

    pub fn current_host_fence(&self) -> Result<RecoveryHostFence, RecoveryStoreError> {
        self.current_fence()?
            .ok_or(RecoveryStoreError::HostFenceRequired)
    }
}
