// SPDX-License-Identifier: MIT

use sts2_gateway::{RecoveryHostLeaseState, RecoveryLease, RecoveryStoreError};

use super::super::super::host_lease_control::grant_digest;
use super::super::{RuntimeService, host_lease_helpers::grant_value};

impl RuntimeService {
    pub(in super::super) fn host_lease_ready(
        &self,
        lease_id: &str,
    ) -> Result<bool, RecoveryStoreError> {
        let Some(lease) = self
            .recovery_lease
            .as_ref()
            .filter(|lease| lease.lease_id == lease_id)
        else {
            return Ok(false);
        };
        self.active_host_grant_matches(lease)
    }

    /// Readiness binds the acknowledged durable installation to the complete current grant,
    /// not just an Installed state label. It grants no historical mutation permission.
    pub(in super::super) fn active_host_grant_matches(
        &self,
        lease: &RecoveryLease,
    ) -> Result<bool, RecoveryStoreError> {
        let store = self
            .recovery
            .as_ref()
            .ok_or(RecoveryStoreError::PersistenceUnavailable)?;
        let (Some(boot), Some(fence)) = (self.recovery_boot.as_ref(), self.recovery_fence.as_ref())
        else {
            return Ok(false);
        };
        let Some(binding) = store.host_lease_binding(&lease.lease_id)? else {
            return Ok(false);
        };
        if binding.state != RecoveryHostLeaseState::Installed
            || binding.lease_id != lease.lease_id
            || binding.installation_id.is_none()
            || binding.host_install_generation == 0
            || binding.host_ack_message_id.is_none()
            || binding.host_ack_recorded_at_millis.is_none()
            || store.current_host_fence()? != *fence
        {
            return Ok(false);
        }
        let grant = grant_value(
            boot,
            fence,
            lease,
            &self.config.caller_id,
            &self.config.session_id,
        );
        let digest = grant_digest(&grant).map_err(|_| {
            RecoveryStoreError::ContractMismatch(String::from(
                "current host grant cannot be canonicalized",
            ))
        })?;
        if binding.grant_digest.as_deref() != Some(digest.as_str()) {
            return Ok(false);
        }
        let Some(cached) = self.recovery_host_grant.as_ref() else {
            return Ok(false);
        };
        Ok(
            Some(cached.installation_id.as_str()) == binding.installation_id.as_deref()
                && cached.grant_digest == digest
                && cached.grant == grant,
        )
    }
}
