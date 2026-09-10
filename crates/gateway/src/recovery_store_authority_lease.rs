// SPDX-License-Identifier: MIT

use super::super::recovery_types::{
    RecoveryBootState, RecoveryLease, RecoveryStoreError, validate_identity, validate_uuid,
    validate_uuid_v4, validate_wire,
};
use super::super::{random_token, random_uuid, sha256_hex};
use super::GatewayRecoveryStore;
use super::RecoveryLeaseRequest;

impl GatewayRecoveryStore {
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
        if request.ttl_seconds != self.config.lease_ttl_seconds
            || request.renewal_interval_seconds != self.config.lease_renewal_interval_seconds
        {
            return Err(RecoveryStoreError::InvalidInput(
                "lease policy differs from configured store policy".to_owned(),
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
}
