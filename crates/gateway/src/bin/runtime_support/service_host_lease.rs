// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::{
    RecoveryBootContext, RecoveryHostFence, RecoveryHostLeaseState, RecoveryLease,
    RecoveryLeaseProof,
};

use super::super::host_lease_control::{
    HostLeaseKind, ack_context, ack_status, grant_digest, request_frame,
};
use super::host_lease_helpers::{
    grant_value, lease_deadline, map_frame_error, map_store_error, validate_ack,
};
use super::{HostLeaseGrant, HttpRequest, RuntimeService, json_error, safe_identity};

#[path = "service_host_lease_readiness.rs"]
mod readiness;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HostLeaseFailure {
    pub(super) status: u16,
    pub(super) code: &'static str,
}

impl HostLeaseFailure {
    pub(super) const fn configuration() -> Self {
        Self {
            status: 500,
            code: "recovery_host_lease_configuration_invalid",
        }
    }

    pub(super) const fn unknown() -> Self {
        Self {
            status: 503,
            code: "recovery_host_lease_outcome_unknown",
        }
    }

    pub(super) const fn expired() -> Self {
        Self {
            status: 410,
            code: "recovery_lease_expired",
        }
    }

    pub(super) const fn invalid_response() -> Self {
        Self {
            status: 502,
            code: "recovery_host_lease_response_invalid",
        }
    }

    pub(super) fn body(self) -> (u16, Vec<u8>) {
        (self.status, json_error(self.code))
    }
}

impl RuntimeService {
    /// A revoke request may be retried after the host response was lost. The
    /// local lease is already revoked at that point, so the ordinary mutation
    /// lease check cannot be used to authenticate the retry. Keep the retry
    /// narrow: it must carry the same in-memory lease identity and a durable
    /// pending-host-revoke state.
    pub(super) fn pending_host_revoke_matches(&mut self, request: &HttpRequest) -> bool {
        self.check_recovery_deadline();
        let Some(lease) = self.recovery_lease.as_ref() else {
            return false;
        };
        let pending = self
            .recovery
            .as_ref()
            .and_then(|store| store.host_lease_binding(&lease.lease_id).ok().flatten())
            .is_some_and(|binding| binding.state == RecoveryHostLeaseState::PendingHostRevoke);
        if !pending {
            return false;
        }
        let expected_epoch = lease.lease_epoch.to_string();
        let expected = [
            ("x-sts2-instance-id", lease.instance_id.as_str()),
            ("x-sts2-caller-id", self.config.caller_id.as_str()),
            ("x-sts2-session-id", self.config.session_id.as_str()),
            ("x-mcp-session-id", self.config.mcp_session_id.as_str()),
            ("x-sts2-lease-id", lease.lease_id.as_str()),
            ("x-sts2-lease-epoch", expected_epoch.as_str()),
        ];
        request
            .headers
            .iter()
            .find(|(name, _)| name.as_str() == "x-sts2-correlation-id")
            .and_then(|(_, value)| safe_identity(value).then_some(()))
            .is_some()
            && expected
                .iter()
                .all(|(name, value)| request.headers.get(*name).map(String::as_str) == Some(*value))
    }

    pub(super) fn install_host_lease(
        &mut self,
        boot: &RecoveryBootContext,
        fence: &RecoveryHostFence,
        lease: &RecoveryLease,
        correlation: &str,
    ) -> Result<RecoveryLease, HostLeaseFailure> {
        let result = self.install_host_lease_inner(boot, fence, lease, correlation);
        if let Err(failure) = result {
            super::allocation_cleanup::failed_install(self, lease, failure);
        }
        result
    }

    fn install_host_lease_inner(
        &mut self,
        boot: &RecoveryBootContext,
        fence: &RecoveryHostFence,
        lease: &RecoveryLease,
        correlation: &str,
    ) -> Result<RecoveryLease, HostLeaseFailure> {
        let send_started = std::time::Instant::now();
        let (installation_id, grant, digest, already_installed) =
            self.prepare_host_install(boot, fence, lease)?;
        let received_at = self.recovery_now_millis();
        self.establish_install_deadline(lease, send_started, received_at)?;
        if already_installed {
            self.recovery_host_grant = Some(HostLeaseGrant {
                installation_id,
                grant_digest: digest,
                grant,
            });
            self.activate_recovery_lease(lease.clone())?;
            return Ok(lease.clone());
        }
        // Retain the exact grant before any configuration lookup or frame
        // serialization can fail. The durable pending row and this cache
        // then describe one retryable installation attempt.
        self.recovery_host_grant = Some(HostLeaseGrant {
            installation_id: installation_id.clone(),
            grant_digest: digest.clone(),
            grant: grant.clone(),
        });
        let frame = request_frame(
            HostLeaseKind::Install,
            &self.config.caller_id,
            correlation,
            json!({
                "installation_id": installation_id,
                "grant": grant,
                "grant_digest": digest,
            }),
            &self.config.host_lease_key,
        )
        .map_err(map_frame_error)?;
        let response = self.forward_host_lease(HostLeaseKind::Install, &frame)?;
        let response_received_at = self.recovery_now_millis();
        self.establish_install_deadline(lease, send_started, response_received_at)?;
        let status = ack_status(&response, HostLeaseKind::Install)
            .ok_or_else(HostLeaseFailure::invalid_response)?;
        let ack = ack_context(&response).ok_or_else(HostLeaseFailure::invalid_response)?;
        validate_ack(
            HostLeaseKind::Install,
            &ack,
            lease,
            fence,
            &installation_id,
            &digest,
            None,
        )?;
        if let Err(error) = self
            .recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .complete_host_lease_install(
                &lease.lease_id,
                &installation_id,
                &digest,
                ack.host_install_generation,
                &ack.message_id,
                ack.recorded_at,
            )
        {
            return Err(map_store_error(error));
        }
        #[cfg(test)]
        super::allocation_cleanup::apply_expiry_after_install(self);
        self.recovery_host_grant = Some(HostLeaseGrant {
            installation_id,
            grant_digest: digest,
            grant,
        });
        if status == "INSTALLED" || status == "DUPLICATE" {
            self.activate_recovery_lease(lease.clone())?;
            Ok(lease.clone())
        } else {
            Err(HostLeaseFailure::invalid_response())
        }
    }

    pub(super) fn renew_host_lease(
        &mut self,
        proof: &RecoveryLeaseProof,
        sequence: u64,
        correlation: &str,
    ) -> Result<RecoveryLease, HostLeaseFailure> {
        let send_started = std::time::Instant::now();
        let now = self.recovery_now_millis();
        let candidate = self
            .recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .prepare_host_lease_renew(proof, sequence, now)
            .map_err(map_store_error)?;
        let renewal_deadline = lease_deadline(
            candidate.expires_at_millis,
            candidate.ttl_seconds,
            send_started,
            now,
        )?;
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
            .host_lease_binding(&candidate.lease_id)
            .map_err(map_store_error)?
            .ok_or(HostLeaseFailure::configuration())?;
        let installation_id = binding
            .installation_id
            .clone()
            .ok_or(HostLeaseFailure::unknown())?;
        let grant = grant_value(
            &boot,
            &fence,
            &candidate,
            &self.config.caller_id,
            &self.config.session_id,
        );
        let digest = grant_digest(&grant).map_err(map_frame_error)?;
        if binding.grant_digest.as_deref() != Some(digest.as_str()) {
            self.recovery
                .as_mut()
                .ok_or(HostLeaseFailure::configuration())?
                .set_host_lease_grant_digest(&candidate.lease_id, &installation_id, &digest)
                .map_err(map_store_error)?;
        }
        let frame = request_frame(
            HostLeaseKind::Renew,
            &self.config.caller_id,
            correlation,
            json!({
                "installation_id": installation_id,
                "grant": grant,
                "grant_digest": digest,
                "renew_sequence": sequence,
            }),
            &self.config.host_lease_key,
        )
        .map_err(map_frame_error)?;
        let response = self.forward_host_lease(HostLeaseKind::Renew, &frame)?;
        let status = ack_status(&response, HostLeaseKind::Renew)
            .ok_or_else(HostLeaseFailure::invalid_response)?;
        let ack = ack_context(&response).ok_or_else(HostLeaseFailure::invalid_response)?;
        validate_ack(
            HostLeaseKind::Renew,
            &ack,
            &candidate,
            &fence,
            &installation_id,
            &digest,
            Some((sequence, candidate.expires_at_millis)),
        )?;
        if status != "RENEWED" && status != "RENEW_DUPLICATE" {
            return Err(HostLeaseFailure::invalid_response());
        }
        if std::time::Instant::now() >= renewal_deadline {
            return Err(HostLeaseFailure::expired());
        }
        self.recovery
            .as_mut()
            .ok_or(HostLeaseFailure::configuration())?
            .complete_host_lease_renew(
                &candidate.lease_id,
                &installation_id,
                &digest,
                sequence,
                candidate.expires_at_millis,
                ack.host_install_generation,
                &ack.message_id,
                ack.recorded_at,
            )
            .map_err(map_store_error)?;
        self.recovery_host_grant = Some(HostLeaseGrant {
            installation_id,
            grant_digest: digest,
            grant,
        });
        self.recovery_lease_deadline = Some(renewal_deadline);
        self.recovery_lease_deadline_lease_id = Some(candidate.lease_id.clone());
        self.activate_recovery_lease(candidate.clone())?;
        Ok(candidate)
    }
}

#[cfg(test)]
#[path = "service_host_lease_tests.rs"]
mod tests;
