// SPDX-License-Identifier: MIT

use super::super::GatewayRecoveryStore;
use super::super::recovery_types::{
    RecoveryBootState, RecoveryLease, RecoveryLeaseProof, RecoveryLeaseState, RecoveryStoreError,
    validate_identity, validate_uuid, validate_uuid_v4, validate_wire,
};

impl GatewayRecoveryStore {
    pub fn renew_lease(
        &mut self,
        proof: &RecoveryLeaseProof,
        renew_sequence: u64,
        now_millis: u64,
    ) -> Result<RecoveryLease, RecoveryStoreError> {
        validate_wire(renew_sequence, "renew_sequence")?;
        validate_wire(now_millis, "now_millis")?;
        if renew_sequence == 0 {
            return Err(RecoveryStoreError::InvalidInput(
                "renew_sequence must be positive".to_owned(),
            ));
        }
        validate_lease_proof_shape(proof)?;
        let authority = self
            .current_authority()?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if authority.state != RecoveryBootState::Ready
            || authority.boot_id != proof.boot_id
            || authority.instance_incarnation != proof.instance_incarnation
            || authority.authority_generation != proof.authority_generation
            || authority.deployment_id != proof.deployment_id
            || authority.instance_id != proof.instance_id
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        let lease = self
            .lease_by_id_with_token(&proof.lease_id, &proof.fence_token)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        check_lease_identity(&lease, proof)?;
        match lease_state(self, &proof.lease_id)? {
            RecoveryLeaseState::Active => {}
            RecoveryLeaseState::Expired => return Err(RecoveryStoreError::LeaseExpired),
            RecoveryLeaseState::Revoked => return Err(RecoveryStoreError::LeaseRevoked),
        }
        if lease.expires_at_millis <= now_millis {
            self.conn
                .execute(
                    "UPDATE leases SET status = 'EXPIRED', revoked_reason = 'ttl'
                     WHERE lease_id = ?1 AND status = 'ACTIVE'",
                    [proof.lease_id.as_str()],
                )
                .map_err(super::map_sql_error)?;
            return Err(RecoveryStoreError::LeaseExpired);
        }
        if renew_sequence <= lease.last_renew_sequence {
            return Err(RecoveryStoreError::StaleLease);
        }
        let expires = now_millis
            .checked_add(
                lease
                    .ttl_seconds
                    .checked_mul(1_000)
                    .ok_or(RecoveryStoreError::CounterExhausted)?,
            )
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        validate_wire(expires, "expires_at_millis")?;
        let changed = self
            .conn
            .execute(
                "UPDATE leases SET expires_at_millis = ?1, last_renew_sequence = ?2
                 WHERE lease_id = ?3 AND status = 'ACTIVE' AND last_renew_sequence < ?2",
                rusqlite::params![expires as i64, renew_sequence as i64, proof.lease_id],
            )
            .map_err(super::map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(RecoveryLease {
            fence_token: proof.fence_token.clone(),
            expires_at_millis: expires,
            last_renew_sequence: renew_sequence,
            ..lease
        })
    }

    pub fn revoke_lease(
        &mut self,
        proof: &RecoveryLeaseProof,
        reason: &str,
    ) -> Result<(), RecoveryStoreError> {
        validate_lease_proof_shape(proof)?;
        let lease = self
            .lease_by_id_with_token(&proof.lease_id, &proof.fence_token)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        check_lease_identity(&lease, proof)?;
        validate_identity("revoke_reason", reason)?;
        let changed = self
            .conn
            .execute(
                "UPDATE leases SET status = 'REVOKED', revoked_reason = ?1
                 WHERE lease_id = ?2 AND status = 'ACTIVE'",
                rusqlite::params![reason, proof.lease_id],
            )
            .map_err(super::map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::LeaseRevoked);
        }
        Ok(())
    }

    /// Expires every active lease at or before the supplied wall-clock audit
    /// timestamp. The update is one SQLite statement so a renewal loop cannot
    /// observe a partially expired set of leases.
    pub fn expire_leases(&mut self, now_millis: u64) -> Result<usize, RecoveryStoreError> {
        validate_wire(now_millis, "now_millis")?;
        self.conn
            .execute(
                "UPDATE leases SET status = 'EXPIRED', revoked_reason = 'ttl'
                 WHERE status = 'ACTIVE' AND expires_at_millis <= ?1",
                [now_millis as i64],
            )
            .map_err(super::map_sql_error)
    }

    pub fn validate_lease(
        &mut self,
        proof: &RecoveryLeaseProof,
        now_millis: u64,
    ) -> Result<(), RecoveryStoreError> {
        let lease = self.ensure_context(proof, now_millis)?;
        if lease_state(self, &lease.lease_id)? != RecoveryLeaseState::Active {
            return Err(RecoveryStoreError::LeaseRevoked);
        }
        Ok(())
    }
}

fn lease_state(
    store: &GatewayRecoveryStore,
    lease_id: &str,
) -> Result<RecoveryLeaseState, RecoveryStoreError> {
    let state = store
        .conn
        .query_row(
            "SELECT status FROM leases WHERE lease_id = ?1",
            [lease_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(super::map_sql_error)?;
    match state.as_str() {
        "ACTIVE" => Ok(RecoveryLeaseState::Active),
        "EXPIRED" => Ok(RecoveryLeaseState::Expired),
        "REVOKED" => Ok(RecoveryLeaseState::Revoked),
        _ => Err(RecoveryStoreError::Corrupt(
            "unknown lease state".to_owned(),
        )),
    }
}

fn check_lease_identity(
    lease: &RecoveryLease,
    proof: &RecoveryLeaseProof,
) -> Result<(), RecoveryStoreError> {
    if lease.deployment_id != proof.deployment_id
        || lease.instance_id != proof.instance_id
        || lease.instance_incarnation != proof.instance_incarnation
        || lease.boot_id != proof.boot_id
        || lease.authority_generation != proof.authority_generation
        || lease.lease_epoch != proof.lease_epoch
    {
        return Err(RecoveryStoreError::StaleLease);
    }
    Ok(())
}

fn validate_lease_proof_shape(proof: &RecoveryLeaseProof) -> Result<(), RecoveryStoreError> {
    validate_uuid("deployment_id", &proof.deployment_id)?;
    validate_uuid("instance_id", &proof.instance_id)?;
    validate_uuid_v4("instance_incarnation", &proof.instance_incarnation)?;
    validate_uuid_v4("boot_id", &proof.boot_id)?;
    validate_uuid_v4("lease_id", &proof.lease_id)?;
    super::super::recovery_types::validate_token("fence_token", &proof.fence_token)?;
    validate_wire(proof.authority_generation, "authority_generation")?;
    validate_wire(proof.lease_epoch, "lease_epoch")
}
