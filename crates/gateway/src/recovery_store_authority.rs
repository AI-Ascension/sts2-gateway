// SPDX-License-Identifier: MIT

use rusqlite::OptionalExtension;

use super::super::random_uuid;
use super::super::recovery_types::{
    MAX_WIRE_INTEGER, RecoveryBootContext, RecoveryBootState, RecoveryHostFence,
    RecoveryReleaseSet, RecoveryStoreError, RecoveryUncertaintyReason, validate_uuid,
    validate_uuid_v4, validate_wire,
};
use super::GatewayRecoveryStore;

impl GatewayRecoveryStore {
    pub fn start_boot(
        &mut self,
        deployment_id: &str,
        instance_id: &str,
        release: RecoveryReleaseSet,
        now_millis: u64,
    ) -> Result<RecoveryBootContext, RecoveryStoreError> {
        self.start_boot_with_incarnation(
            deployment_id,
            instance_id,
            &random_uuid(),
            release,
            now_millis,
        )
    }

    pub fn start_boot_with_incarnation(
        &mut self,
        deployment_id: &str,
        instance_id: &str,
        instance_incarnation: &str,
        release: RecoveryReleaseSet,
        now_millis: u64,
    ) -> Result<RecoveryBootContext, RecoveryStoreError> {
        validate_uuid("deployment_id", deployment_id)?;
        validate_uuid("instance_id", instance_id)?;
        validate_uuid_v4("instance_incarnation", instance_incarnation)?;
        validate_wire(now_millis, "now_millis")?;
        release.validate()?;
        let boot_id = random_uuid();
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
                instance_incarnation,
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
            instance_incarnation: instance_incarnation.to_owned(),
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
        self.complete_host_fence_from_host(
            boot,
            &random_uuid(),
            boot.authority_generation,
            now_millis,
        )
    }

    pub fn complete_host_fence_from_host(
        &mut self,
        boot: &RecoveryBootContext,
        host_fence_id: &str,
        fence_generation: u64,
        now_millis: u64,
    ) -> Result<RecoveryHostFence, RecoveryStoreError> {
        validate_uuid_v4("host_fence_id", host_fence_id)?;
        validate_wire(fence_generation, "fence_generation")?;
        validate_wire(now_millis, "now_millis")?;
        let current = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if current != *boot || current.state != RecoveryBootState::FenceRequired {
            return Err(RecoveryStoreError::StaleLease);
        }
        let fence = RecoveryHostFence {
            host_fence_id: host_fence_id.to_owned(),
            deployment_id: boot.deployment_id.clone(),
            instance_id: boot.instance_id.clone(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            fence_generation,
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

    pub fn current_boot(&self) -> Result<RecoveryBootContext, RecoveryStoreError> {
        self.current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)
    }

    pub fn current_host_fence(&self) -> Result<RecoveryHostFence, RecoveryStoreError> {
        self.current_fence()?
            .ok_or(RecoveryStoreError::HostFenceRequired)
    }
}
