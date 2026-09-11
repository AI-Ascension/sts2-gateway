// SPDX-License-Identifier: MIT

use super::super::random_uuid;
use super::super::recovery_types::{
    MAX_WIRE_INTEGER, RecoveryBootContext, RecoveryBootState, RecoveryReleaseSet,
    RecoveryStoreError, RecoveryUncertaintyReason, validate_uuid, validate_wire,
};
use super::GatewayRecoveryStore;

impl GatewayRecoveryStore {
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
            "UPDATE leases SET host_state = 'RESTART_INVALIDATED'
             WHERE host_state IN
                ('PENDING_HOST_INSTALL', 'INSTALLED', 'PENDING_HOST_RENEW', 'PENDING_HOST_REVOKE')",
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
