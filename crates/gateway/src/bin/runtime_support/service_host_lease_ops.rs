// SPDX-License-Identifier: MIT

//! Host lease revoke, installation preparation, and transport operations.

use std::time::Instant;

use serde_json::{Value, json};
use sts2_gateway::{RecoveryBootContext, RecoveryHostFence, RecoveryHostLeaseState, RecoveryLease};
use uuid::Uuid;

use super::super::host_lease_control::{
    HostLeaseFrameError, HostLeaseKind, ack_context, ack_status, grant_digest, parse_response,
    request_frame,
};
use super::super::recovery_control::{HttpRecoveryControlForwarder, RecoveryControlTransportFault};
use super::RuntimeService;
use super::host_lease::HostLeaseFailure;
use super::host_lease_helpers::{
    frame_correlation, grant_value, lease_deadline, map_frame_error, map_store_error, validate_ack,
};

#[path = "service_host_lease_ops_mapping.rs"]
mod mapping;
use mapping::{map_host_lease_response_error, map_host_lease_transport_error};

impl RuntimeService {
    pub(super) fn revoke_host_lease(
        &mut self,
        proof: &sts2_gateway::RecoveryLeaseProof,
        reason: &str,
        correlation: &str,
    ) -> Result<(), HostLeaseFailure> {
        if reason != "shutdown" {
            self.allocation_cleanup_lease_id = None;
        }
        let lease = self
            .recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .begin_host_lease_revoke(proof, reason)
            .map_err(map_store_error)?;
        // The gateway has durably revoked its local authority before the host
        // request is written. Keep the lease in memory only to reconcile a
        // lost revoke acknowledgment; all ordinary mutation admission is
        // closed immediately.
        self.lease_active = false;
        self.lease_revoked = true;
        self.recovery_lease_deadline = None;
        let boot = self
            .recovery_boot
            .clone()
            .ok_or(HostLeaseFailure::configuration())?;
        let fence = self
            .recovery_fence
            .clone()
            .ok_or(HostLeaseFailure::configuration())?;
        let binding = self
            .recovery
            .as_ref()
            .ok_or(HostLeaseFailure::configuration())?
            .host_lease_binding(&lease.lease_id)
            .map_err(map_store_error)?
            .ok_or(HostLeaseFailure::configuration())?;
        let installation_id = binding
            .installation_id
            .clone()
            .ok_or(HostLeaseFailure::unknown())?;
        let grant = grant_value(
            &boot,
            &fence,
            &lease,
            &self.config.caller_id,
            &self.config.session_id,
        );
        let digest = grant_digest(&grant).map_err(map_frame_error)?;
        if binding.grant_digest.as_deref() != Some(digest.as_str()) {
            return Err(HostLeaseFailure::unknown());
        }
        let frame = request_frame(
            HostLeaseKind::Revoke,
            &self.config.caller_id,
            correlation,
            json!({
                "installation_id": installation_id,
                "grant": grant,
                "grant_digest": digest,
                "reason": reason,
            }),
            &self.config.host_lease_key,
        )
        .map_err(map_frame_error)?;
        let response = self.forward_host_lease(HostLeaseKind::Revoke, &frame)?;
        let status = ack_status(&response, HostLeaseKind::Revoke)
            .ok_or_else(HostLeaseFailure::invalid_response)?;
        let ack = ack_context(&response).ok_or_else(HostLeaseFailure::invalid_response)?;
        validate_ack(
            HostLeaseKind::Revoke,
            &ack,
            &lease,
            &fence,
            &installation_id,
            &digest,
            None,
        )?;
        if status != "REVOKED" && status != "REVOKE_DUPLICATE" {
            return Err(HostLeaseFailure::invalid_response());
        }
        self.recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .complete_host_lease_revoke(
                &lease.lease_id,
                &installation_id,
                &digest,
                ack.host_install_generation,
                &ack.message_id,
                ack.recorded_at,
            )
            .map_err(map_store_error)?;
        let may_allocate_fresh = self.allocation_cleanup_lease_id.as_deref()
            == Some(lease.lease_id.as_str())
            && !self.shutdown_requested;
        self.allocation_cleanup_lease_id = None;
        self.recovery_lease = None;
        self.recovery_lease_deadline = None;
        self.recovery_lease_deadline_lease_id = None;
        self.recovery_host_grant = None;
        self.lease_active = false;
        self.lease_revoked = !may_allocate_fresh;
        Ok(())
    }

    pub(super) fn prepare_host_install(
        &mut self,
        boot: &RecoveryBootContext,
        fence: &RecoveryHostFence,
        lease: &RecoveryLease,
    ) -> Result<(String, Value, String, bool), HostLeaseFailure> {
        let grant = grant_value(
            boot,
            fence,
            lease,
            &self.config.caller_id,
            &self.config.session_id,
        );
        let digest = grant_digest(&grant).map_err(map_frame_error)?;
        let existing = self
            .recovery
            .as_ref()
            .ok_or(HostLeaseFailure::configuration())?
            .host_lease_binding(&lease.lease_id)
            .map_err(map_store_error)?;
        let installation_id = match existing
            .as_ref()
            .and_then(|binding| binding.installation_id.clone())
        {
            Some(id) => id,
            None => Uuid::new_v4().to_string(),
        };
        if let Some(binding) = existing.as_ref() {
            if binding.grant_digest.as_deref() != Some(digest.as_str())
                && binding.state != RecoveryHostLeaseState::Uninstalled
            {
                return Err(HostLeaseFailure::unknown());
            }
            if binding.state == RecoveryHostLeaseState::Installed {
                if binding.grant_digest.as_deref() != Some(digest.as_str()) {
                    return Err(HostLeaseFailure::unknown());
                }
                return Ok((installation_id, grant, digest, true));
            }
            if binding.state == RecoveryHostLeaseState::RestartInvalidated {
                return Err(HostLeaseFailure::unknown());
            }
            if binding.state == RecoveryHostLeaseState::PendingHostInstall
                && let Some(cached) = self.recovery_host_grant.as_ref()
            {
                if cached.installation_id != installation_id
                    || cached.grant_digest != digest
                    || cached.grant != grant
                {
                    return Err(HostLeaseFailure::unknown());
                }
            } else if binding.state == RecoveryHostLeaseState::PendingHostInstall {
                return Err(HostLeaseFailure::unknown());
            }
        }
        let now = self.recovery_now_millis();
        self.recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .prepare_host_lease_install(
                &lease.lease_id,
                &installation_id,
                &digest,
                &fence.host_fence_id,
                fence.fence_generation,
                now,
            )
            .map_err(map_store_error)?;
        Ok((installation_id, grant, digest, false))
    }

    pub(super) fn forward_host_lease(
        &self,
        kind: HostLeaseKind,
        frame: &[u8],
    ) -> Result<Value, HostLeaseFailure> {
        let forwarder =
            HttpRecoveryControlForwarder::new(&self.config.mod_address, &self.config.mod_token);
        let response = forwarder
            .forward_host_lease_frame(kind, frame, &self.config.host_lease_key)
            .map_err(map_host_lease_transport_error)?;
        if response.status != 200 {
            return Err(if response.status >= 500 {
                HostLeaseFailure::unknown()
            } else {
                HostLeaseFailure {
                    status: response.status,
                    code: "recovery_host_lease_rejected",
                }
            });
        }
        let kind_response = parse_response(
            &response.body,
            kind,
            &self.config.host_lease_key,
            &self.config.host_principal_id,
        )
        .map_err(map_host_lease_response_error)?;
        if kind_response["correlation_id"] != frame_correlation(frame) {
            return Err(HostLeaseFailure {
                status: 409,
                code: "recovery_host_lease_correlation_mismatch",
            });
        }
        Ok(kind_response)
    }

    pub(super) fn activate_recovery_lease(
        &mut self,
        lease: RecoveryLease,
    ) -> Result<(), HostLeaseFailure> {
        let now = self.recovery_now_millis();
        let Some(deadline) = self.recovery_lease_deadline else {
            return Err(HostLeaseFailure::expired());
        };
        if now >= lease.expires_at_millis || Instant::now() >= deadline {
            self.recovery_lease_deadline = None;
            self.lease_active = false;
            return Err(HostLeaseFailure::expired());
        }
        self.recovery_lease = Some(lease);
        self.lease_active = true;
        self.lease_revoked = false;
        Ok(())
    }

    pub(super) fn establish_install_deadline(
        &mut self,
        lease: &RecoveryLease,
        send_started: Instant,
        received_at: u64,
    ) -> Result<(), HostLeaseFailure> {
        let same_lease =
            self.recovery_lease_deadline_lease_id.as_deref() == Some(lease.lease_id.as_str());
        if !same_lease {
            self.recovery_lease_deadline = None;
            self.recovery_lease_deadline_lease_id = None;
        }
        if let Some(deadline) = self.recovery_lease_deadline {
            if received_at >= lease.expires_at_millis || Instant::now() >= deadline {
                self.recovery_lease_deadline = None;
                return Err(HostLeaseFailure::expired());
            }
            return Ok(());
        }
        if same_lease {
            return Err(HostLeaseFailure::expired());
        }
        self.recovery_lease_deadline_lease_id = Some(lease.lease_id.clone());
        self.recovery_lease_deadline = Some(lease_deadline(
            lease.expires_at_millis,
            lease.ttl_seconds,
            send_started,
            received_at,
        )?);
        Ok(())
    }
}

#[cfg(test)]
#[path = "service_host_lease_ops_tests.rs"]
mod tests;
