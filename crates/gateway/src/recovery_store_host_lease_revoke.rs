// SPDX-License-Identifier: MIT

//! Durable host lease revoke and restart invalidation operations.

use super::super::recovery_types::{
    RecoveryHostLeaseBinding, RecoveryHostLeaseState, RecoveryLease, RecoveryLeaseProof,
    RecoveryStoreError, validate_identity,
};
use super::host_lease::HostAckKind;
use super::host_lease_ack::{check_lease_identity, validate_lease_proof_shape};
use super::{GatewayRecoveryStore, map_sql_error};
impl GatewayRecoveryStore {
    /// Disables local mutation immediately and leaves a durable pending host
    /// revoke until the matching acknowledgment arrives.
    pub fn begin_host_lease_revoke(
        &mut self,
        proof: &RecoveryLeaseProof,
        reason: &str,
    ) -> Result<RecoveryLease, RecoveryStoreError> {
        validate_lease_proof_shape(proof)?;
        validate_identity("revoke_reason", reason)?;
        let lease = self
            .lease_by_id_with_token(&proof.lease_id, &proof.fence_token)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        check_lease_identity(&lease, proof)?;
        let binding = self
            .host_lease_binding(&proof.lease_id)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        if !matches!(
            binding.state,
            RecoveryHostLeaseState::Installed | RecoveryHostLeaseState::PendingHostRevoke
        ) {
            return Err(RecoveryStoreError::HostFenceRequired);
        }
        if binding.state == RecoveryHostLeaseState::PendingHostRevoke {
            return Ok(lease);
        }
        let changed = self
            .conn
            .execute(
                "UPDATE leases SET status = 'REVOKED', revoked_reason = ?1,
                        host_state = ?2
                 WHERE lease_id = ?3 AND status = 'ACTIVE' AND host_state = 'INSTALLED'",
                rusqlite::params![
                    reason,
                    RecoveryHostLeaseState::PendingHostRevoke.as_str(),
                    proof.lease_id,
                ],
            )
            .map_err(map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(lease)
    }

    pub fn complete_host_lease_revoke(
        &mut self,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
        host_install_generation: u64,
        ack_message_id: &str,
        ack_recorded_at_millis: u64,
    ) -> Result<RecoveryHostLeaseBinding, RecoveryStoreError> {
        self.complete_host_ack(
            HostAckKind::Revoke,
            lease_id,
            installation_id,
            grant_digest,
            host_install_generation,
            0,
            ack_message_id,
            ack_recorded_at_millis,
        )
    }

    pub fn invalidate_host_leases_for_restart(&mut self) -> Result<usize, RecoveryStoreError> {
        self.conn
            .execute(
                "UPDATE leases SET host_state = ?1
                 WHERE host_state IN ('PENDING_HOST_INSTALL', 'INSTALLED',
                                      'PENDING_HOST_RENEW', 'PENDING_HOST_REVOKE')",
                [RecoveryHostLeaseState::RestartInvalidated.as_str()],
            )
            .map_err(map_sql_error)
    }
}
