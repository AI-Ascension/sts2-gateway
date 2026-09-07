// SPDX-License-Identifier: MIT

//! Durable host acknowledgment commits and protected-store validation helpers.

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryHostLeaseBinding, RecoveryHostLeaseState, RecoveryLease, RecoveryLeaseProof,
    RecoveryStoreError, validate_token, validate_uuid, validate_uuid_v4, validate_wire,
};
use super::super::sha256_hex;
use super::host_lease::HostAckKind;
use super::{GatewayRecoveryStore, map_sql_error, row_u64};

pub(super) fn validate_digest(value: &str, name: &str) -> Result<(), RecoveryStoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} must be a lowercase SHA-256 digest"
        )));
    }
    Ok(())
}

impl GatewayRecoveryStore {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn complete_host_ack(
        &mut self,
        kind: HostAckKind,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
        host_install_generation: u64,
        renew_sequence: u64,
        ack_message_id: &str,
        ack_recorded_at_millis: u64,
    ) -> Result<RecoveryHostLeaseBinding, RecoveryStoreError> {
        validate_uuid_v4("lease_id", lease_id)?;
        validate_uuid_v4("installation_id", installation_id)?;
        validate_digest(grant_digest, "grant_digest")?;
        validate_wire(host_install_generation, "host_install_generation")?;
        validate_wire(renew_sequence, "renew_sequence")?;
        validate_uuid_v4("ack_message_id", ack_message_id)?;
        validate_wire(ack_recorded_at_millis, "ack_recorded_at_millis")?;
        if host_install_generation == 0 {
            return Err(RecoveryStoreError::InvalidInput(
                "host_install_generation must be positive".to_owned(),
            ));
        }
        let row = self
            .conn
            .query_row(
                "SELECT host_state, host_installation_id, host_grant_digest,
                        host_install_generation, host_renew_sequence,
                        host_ack_message_id, host_ack_recorded_at_millis
                 FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| {
                    Ok((
                        RecoveryHostLeaseState::parse(&row.get::<_, String>(0)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row_u64(row, 3)?,
                        row_u64(row, 4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        if row.1.as_deref() != Some(installation_id) || row.2.as_deref() != Some(grant_digest) {
            return Err(RecoveryStoreError::StaleLease);
        }
        if row.0 == kind.state()
            && row.3 == host_install_generation
            && (kind != HostAckKind::Renew || row.4 == renew_sequence)
        {
            // Duplicate acknowledgments replay the original durable ack
            // identity; do not replace it with a later transport message.
            return self
                .host_lease_binding(lease_id)?
                .ok_or(RecoveryStoreError::LeaseNotFound);
        }
        let expected_state = match kind {
            HostAckKind::Install => RecoveryHostLeaseState::PendingHostInstall,
            HostAckKind::Renew => RecoveryHostLeaseState::PendingHostRenew,
            HostAckKind::Revoke => RecoveryHostLeaseState::PendingHostRevoke,
        };
        if row.0 != expected_state {
            return Err(RecoveryStoreError::StaleLease);
        }
        let changed = self
            .conn
            .execute(
                "UPDATE leases SET host_state = ?1, host_install_generation = ?2,
                        host_renew_sequence = ?3, host_ack_message_id = ?4,
                        host_ack_recorded_at_millis = ?5
                 WHERE lease_id = ?6 AND host_state = ?7
                   AND host_installation_id = ?8 AND host_grant_digest = ?9",
                rusqlite::params![
                    kind.state().as_str(),
                    host_install_generation as i64,
                    renew_sequence as i64,
                    ack_message_id,
                    ack_recorded_at_millis as i64,
                    lease_id,
                    expected_state.as_str(),
                    installation_id,
                    grant_digest,
                ],
            )
            .map_err(map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        self.host_lease_binding(lease_id)?
            .ok_or(RecoveryStoreError::LeaseNotFound)
    }
}

pub(super) fn validate_lease_proof_shape(
    proof: &RecoveryLeaseProof,
) -> Result<(), RecoveryStoreError> {
    validate_uuid("deployment_id", &proof.deployment_id)?;
    validate_uuid("instance_id", &proof.instance_id)?;
    validate_uuid_v4("instance_incarnation", &proof.instance_incarnation)?;
    validate_uuid_v4("boot_id", &proof.boot_id)?;
    validate_uuid_v4("lease_id", &proof.lease_id)?;
    validate_token("fence_token", &proof.fence_token)?;
    validate_wire(proof.authority_generation, "authority_generation")?;
    validate_wire(proof.lease_epoch, "lease_epoch")
}

pub(super) fn check_lease_identity(
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

// Keep this module's token-digest helper visibly tied to the protected-store
// invariant. It is used by contract-focused tests and prevents accidental
// future persistence of the plaintext token.
#[allow(dead_code)]
fn protected_token_digest(token: &str) -> String {
    sha256_hex(token.as_bytes())
}
