// SPDX-License-Identifier: MIT

//! Durable gateway-side half of the additive host lease-control contract.
//!
//! The lease table keeps only protected grant material (the existing token
//! digest and the non-secret identity fields).  The plaintext token remains
//! in the caller's in-memory [`RecoveryLease`] for the duration of one
//! process lifetime.  Every method in this module is a commit boundary: a
//! pending state is durable before the caller is allowed to write a host
//! frame, and mutation readiness is restored only by an identity-matching
//! acknowledgment.

use rusqlite::OptionalExtension;

use super::super::recovery_types::{
    RecoveryBootState, RecoveryHostLeaseBinding, RecoveryHostLeaseState, RecoveryStoreError,
    validate_uuid_v4, validate_wire,
};
use super::host_lease_ack::validate_digest;
use super::{GatewayRecoveryStore, map_sql_error, row_u64};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HostAckKind {
    Install,
    Renew,
    Revoke,
}

impl HostAckKind {
    pub(super) const fn state(self) -> RecoveryHostLeaseState {
        match self {
            Self::Install | Self::Renew => RecoveryHostLeaseState::Installed,
            Self::Revoke => RecoveryHostLeaseState::HostRevoked,
        }
    }
}

impl GatewayRecoveryStore {
    /// Returns the protected host binding for a lease.  No token material is
    /// read or reconstructed from the store.
    pub fn host_lease_binding(
        &self,
        lease_id: &str,
    ) -> Result<Option<RecoveryHostLeaseBinding>, RecoveryStoreError> {
        validate_uuid_v4("lease_id", lease_id)?;
        self.conn
            .query_row(
                "SELECT lease_id, host_installation_id, host_grant_digest, host_state,
                        host_install_generation, host_renew_sequence,
                        host_ack_message_id, host_ack_recorded_at_millis
                 FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| {
                    Ok(RecoveryHostLeaseBinding {
                        lease_id: row.get(0)?,
                        installation_id: row.get(1)?,
                        grant_digest: row.get(2)?,
                        state: RecoveryHostLeaseState::parse(&row.get::<_, String>(3)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        host_install_generation: row_u64(row, 4)?,
                        host_renew_sequence: row_u64(row, 5)?,
                        host_ack_message_id: row.get(6)?,
                        host_ack_recorded_at_millis: row
                            .get::<_, Option<i64>>(7)?
                            .map(|value| {
                                u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
                            })
                            .transpose()?,
                    })
                },
            )
            .optional()
            .map_err(map_sql_error)
    }

    pub fn host_lease_is_ready(&self, lease_id: &str) -> Result<bool, RecoveryStoreError> {
        Ok(self
            .host_lease_binding(lease_id)?
            .is_some_and(|binding| binding.state.is_mutation_ready()))
    }

    /// Durably records the installation identity and enters the pending state.
    /// The caller must invoke this before sending a lease-install request.
    pub fn prepare_host_lease_install(
        &mut self,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
        host_fence_id: &str,
        host_fence_generation: u64,
        now_millis: u64,
    ) -> Result<RecoveryHostLeaseBinding, RecoveryStoreError> {
        validate_uuid_v4("lease_id", lease_id)?;
        validate_uuid_v4("installation_id", installation_id)?;
        validate_digest(grant_digest, "grant_digest")?;
        validate_uuid_v4("host_fence_id", host_fence_id)?;
        validate_wire(host_fence_generation, "host_fence_generation")?;
        validate_wire(now_millis, "now_millis")?;
        let tx = self.transaction()?;
        let row = tx
            .query_row(
                "SELECT status, deployment_id, instance_id, instance_incarnation, boot_id,
                        authority_generation, host_installation_id, host_grant_digest,
                        host_state, host_fence_id, host_fence_generation
                 FROM leases WHERE lease_id = ?1",
                [lease_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row_u64(row, 5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        RecoveryHostLeaseState::parse(&row.get::<_, String>(8)?)
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or(RecoveryStoreError::LeaseNotFound)?;
        if row.0 != "ACTIVE" {
            return Err(if row.0 == "EXPIRED" {
                RecoveryStoreError::LeaseExpired
            } else {
                RecoveryStoreError::LeaseRevoked
            });
        }
        let authority = tx
            .query_row(
                "SELECT deployment_id, instance_id, instance_incarnation, boot_id,
                        authority_generation, state, host_fence_id, fence_generation
                 FROM authority WHERE singleton = 1",
                [],
                |value| {
                    Ok((
                        value.get::<_, String>(0)?,
                        value.get::<_, String>(1)?,
                        value.get::<_, String>(2)?,
                        value.get::<_, String>(3)?,
                        row_u64(value, 4)?,
                        value.get::<_, String>(5)?,
                        value.get::<_, Option<String>>(6)?,
                        value.get::<_, Option<i64>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sql_error)?
            .ok_or(RecoveryStoreError::AuthorityNotFound)?;
        if authority.5 != RecoveryBootState::Ready.as_str()
            || authority.0 != row.1
            || authority.1 != row.2
            || authority.2 != row.3
            || authority.3 != row.4
            || authority.4 != row.5
            || authority.6.as_deref() != Some(host_fence_id)
            || authority.7 != Some(host_fence_generation as i64)
        {
            return Err(RecoveryStoreError::StaleLease);
        }
        if let Some(existing) = row.6.as_deref() {
            if existing != installation_id || row.7.as_deref() != Some(grant_digest) {
                return Err(RecoveryStoreError::OperationConflict);
            }
            if matches!(
                row.8,
                RecoveryHostLeaseState::PendingHostInstall | RecoveryHostLeaseState::Installed
            ) {
                tx.commit().map_err(map_sql_error)?;
                return self
                    .host_lease_binding(lease_id)?
                    .ok_or(RecoveryStoreError::LeaseNotFound);
            }
            return Err(RecoveryStoreError::OperationConflict);
        }
        let duplicate = tx
            .query_row(
                "SELECT 1 FROM leases WHERE host_installation_id = ?1 AND lease_id != ?2 LIMIT 1",
                rusqlite::params![installation_id, lease_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_sql_error)?
            .is_some();
        if duplicate {
            return Err(RecoveryStoreError::OperationConflict);
        }
        tx.execute(
            "UPDATE leases SET host_fence_id = ?1, host_fence_generation = ?2,
                    host_installation_id = ?3, host_grant_digest = ?4,
                    host_state = ?5, host_install_generation = 0,
                    host_renew_sequence = 0, host_ack_message_id = NULL,
                    host_ack_recorded_at_millis = NULL, pending_expires_at_millis = NULL,
                    pending_renew_sequence = NULL
             WHERE lease_id = ?6 AND status = 'ACTIVE'",
            rusqlite::params![
                host_fence_id,
                host_fence_generation as i64,
                installation_id,
                grant_digest,
                RecoveryHostLeaseState::PendingHostInstall.as_str(),
                lease_id,
            ],
        )
        .map_err(map_sql_error)?;
        tx.commit().map_err(map_sql_error)?;
        self.host_lease_binding(lease_id)?
            .ok_or(RecoveryStoreError::LeaseNotFound)
    }

    /// Reasserts the same pending install digest before a same-process retry.
    /// This method never creates a new installation identity.
    pub fn set_host_lease_grant_digest(
        &mut self,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
    ) -> Result<(), RecoveryStoreError> {
        validate_uuid_v4("lease_id", lease_id)?;
        validate_uuid_v4("installation_id", installation_id)?;
        validate_digest(grant_digest, "grant_digest")?;
        let changed = self
            .conn
            .execute(
                "UPDATE leases SET host_grant_digest = ?1
                 WHERE lease_id = ?2 AND host_installation_id = ?3
                   AND host_state IN ('PENDING_HOST_INSTALL', 'PENDING_HOST_RENEW', 'PENDING_HOST_REVOKE')",
                rusqlite::params![grant_digest, lease_id, installation_id],
            )
            .map_err(map_sql_error)?;
        if changed != 1 {
            return Err(RecoveryStoreError::StaleLease);
        }
        Ok(())
    }

    pub fn complete_host_lease_install(
        &mut self,
        lease_id: &str,
        installation_id: &str,
        grant_digest: &str,
        host_install_generation: u64,
        ack_message_id: &str,
        ack_recorded_at_millis: u64,
    ) -> Result<RecoveryHostLeaseBinding, RecoveryStoreError> {
        self.complete_host_ack(
            HostAckKind::Install,
            lease_id,
            installation_id,
            grant_digest,
            host_install_generation,
            0,
            ack_message_id,
            ack_recorded_at_millis,
        )
    }
}
