// SPDX-License-Identifier: MIT

//! Durable host lease renewal preparation and acknowledgment commit.

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryHostLeaseBinding, RecoveryHostLeaseState, RecoveryLease, RecoveryLeaseProof,
    RecoveryStoreError, validate_uuid_v4, validate_wire,
};
use super::host_lease_ack::{validate_digest, validate_lease_proof_shape};
use super::{GatewayRecoveryStore, map_sql_error, row_u64};

impl GatewayRecoveryStore {
    /// Prepares a renewal without changing the active local expiry. The
    /// returned candidate is sent to the host; only complete_host_lease_renew
    /// commits it locally.
    pub fn prepare_host_lease_renew(
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
        let current = self.ensure_context(proof, now_millis)?;
        let binding = self
            .host_lease_binding(&proof.lease_id)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        if !matches!(
            binding.state,
            RecoveryHostLeaseState::Installed | RecoveryHostLeaseState::PendingHostRenew
        ) {
            return Err(RecoveryStoreError::HostFenceRequired);
        }
        if binding.state == RecoveryHostLeaseState::PendingHostRenew {
            let (pending_sequence, pending_expires) = self
                .conn
                .query_row(
                    "SELECT pending_renew_sequence, pending_expires_at_millis
                     FROM leases WHERE lease_id = ?1",
                    [proof.lease_id.as_str()],
                    |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
                )
                .map_err(map_sql_error)?;
            if pending_sequence == Some(renew_sequence as i64)
                && let Some(expires) = pending_expires.and_then(|value| u64::try_from(value).ok())
            {
                return Ok(RecoveryLease {
                    expires_at_millis: expires,
                    last_renew_sequence: renew_sequence,
                    ..current
                });
            }
            return Err(RecoveryStoreError::StaleLease);
        }
        if renew_sequence <= current.last_renew_sequence {
            return Err(RecoveryStoreError::StaleLease);
        }
        let ttl_millis = current
            .ttl_seconds
            .checked_mul(1_000)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        let candidate = now_millis
            .checked_add(ttl_millis)
            .ok_or(RecoveryStoreError::CounterExhausted)?;
        let expires = current.expires_at_millis.max(candidate);
        if expires <= current.expires_at_millis {
            return Err(RecoveryStoreError::LeaseExpired);
        }
        self.conn
            .execute(
                "UPDATE leases SET host_state = ?1, pending_expires_at_millis = ?2,
                        pending_renew_sequence = ?3
                 WHERE lease_id = ?4 AND status = 'ACTIVE'
                   AND host_state = 'INSTALLED' AND last_renew_sequence < ?3",
                rusqlite::params![
                    RecoveryHostLeaseState::PendingHostRenew.as_str(),
                    expires as i64,
                    renew_sequence as i64,
                    proof.lease_id,
                ],
            )
            .map_err(map_sql_error)?;
        Ok(RecoveryLease {
            expires_at_millis: expires,
            last_renew_sequence: renew_sequence,
            ..current
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_host_lease_renew(
        &mut self,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
        renew_sequence: u64,
        expires_at_millis: u64,
        host_install_generation: u64,
        ack_message_id: &str,
        ack_recorded_at_millis: u64,
    ) -> Result<RecoveryHostLeaseBinding, RecoveryStoreError> {
        validate_uuid_v4("lease_id", lease_id)?;
        validate_uuid_v4("installation_id", installation_id)?;
        validate_digest(grant_digest, "grant_digest")?;
        validate_wire(renew_sequence, "renew_sequence")?;
        validate_wire(expires_at_millis, "expires_at_millis")?;
        validate_uuid_v4("ack_message_id", ack_message_id)?;
        validate_wire(host_install_generation, "host_install_generation")?;
        validate_wire(ack_recorded_at_millis, "ack_recorded_at_millis")?;
        if renew_sequence == 0 {
            return Err(RecoveryStoreError::InvalidInput(
                "renew_sequence must be positive".to_owned(),
            ));
        }
        if expires_at_millis == 0 || host_install_generation == 0 {
            return Err(RecoveryStoreError::InvalidInput(
                "renewal expiry and host generation must be positive".to_owned(),
            ));
        }
        // The host acknowledgment and the local expiry/sequence update form
        // one commit boundary. A host renewal must never become locally
        // installed while its new expiry remains pending (or vice versa).
        let tx = self.transaction()?;
        let row = tx
            .query_row(
                "SELECT host_state, pending_renew_sequence, pending_expires_at_millis,
                        host_install_generation, host_renew_sequence,
                        host_grant_digest, host_installation_id, expires_at_millis
                 FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| {
                    Ok((
                        RecoveryHostLeaseState::parse(&row.get::<_, String>(0)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row_u64(row, 3)?,
                        row_u64(row, 4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row_u64(row, 7)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        if row.6.as_deref() != Some(installation_id) {
            return Err(RecoveryStoreError::StaleLease);
        }
        if row.0 == RecoveryHostLeaseState::Installed
            && row.3 == host_install_generation
            && row.4 == renew_sequence
            && row.5.as_deref() == Some(grant_digest)
        {
            tx.commit().map_err(map_sql_error)?;
            return self
                .host_lease_binding(lease_id)?
                .ok_or(RecoveryStoreError::LeaseNotFound);
        }
        if row.0 != RecoveryHostLeaseState::PendingHostRenew
            || row.1 != Some(renew_sequence as i64)
            || row.2 != Some(expires_at_millis as i64)
            || row.3 != host_install_generation
            || expires_at_millis <= row.7
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        let changed = tx
            .execute(
                "UPDATE leases SET host_state = ?1, host_install_generation = ?2,
                        host_renew_sequence = ?3, host_ack_message_id = ?4,
                        host_ack_recorded_at_millis = ?5,
                        expires_at_millis = ?6, last_renew_sequence = ?3,
                        pending_expires_at_millis = NULL, pending_renew_sequence = NULL
                 WHERE lease_id = ?7 AND host_state = ?8
                   AND host_installation_id = ?9 AND host_grant_digest = ?10
                   AND pending_renew_sequence = ?3
                   AND pending_expires_at_millis = ?6
                   AND status = 'ACTIVE'",
                rusqlite::params![
                    RecoveryHostLeaseState::Installed.as_str(),
                    host_install_generation as i64,
                    renew_sequence as i64,
                    ack_message_id,
                    ack_recorded_at_millis as i64,
                    expires_at_millis as i64,
                    lease_id,
                    RecoveryHostLeaseState::PendingHostRenew.as_str(),
                    installation_id,
                    grant_digest,
                ],
            )
            .map_err(map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        tx.commit().map_err(map_sql_error)?;
        self.host_lease_binding(lease_id)?
            .ok_or(RecoveryStoreError::LeaseNotFound)
    }
}
