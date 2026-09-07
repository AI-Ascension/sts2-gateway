// SPDX-License-Identifier: MIT

use super::*;
use sts2_gateway::RecoveryHostLeaseState;
use uuid::Uuid;

pub(super) use super::allocation_context::allocation_response;

impl RuntimeService {
    pub(super) fn allocate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        if self.lease_revoked || self.shutdown_requested {
            return (409, json_error("lease_context_revoked"));
        }
        let Ok(allocation) = serde_json::from_slice::<AllocationRequest>(body) else {
            return (400, json_error("allocation_body_invalid"));
        };
        if allocation.instance_id != self.config.instance_id
            || allocation.caller_id != self.config.caller_id
            || allocation.session_id != self.config.session_id
        {
            return (409, json_error("allocation_identity_rejected"));
        }
        if self.recovery.is_some() {
            let Some(boot) = self.recovery_boot.clone() else {
                return (503, json_error("recovery_boot_required"));
            };
            let Some(fence) = self.recovery_fence.clone() else {
                return (503, json_error("recovery_host_fence_required"));
            };
            if let Some(lease) = self.recovery_lease.clone()
                && !self.lease_active
                && lease.boot_id == boot.boot_id
                && self
                    .recovery
                    .as_ref()
                    .and_then(|store| store.host_lease_binding(&lease.lease_id).ok().flatten())
                    .is_some_and(|binding| {
                        binding.state == RecoveryHostLeaseState::PendingHostInstall
                    })
            {
                return match self.install_host_lease(
                    &boot,
                    &fence,
                    &lease,
                    &Uuid::new_v4().to_string(),
                ) {
                    Ok(lease) => allocation_response(self, &lease),
                    Err(error) => error.body(),
                };
            }
            let request = sts2_gateway::RecoveryLeaseRequest {
                deployment_id: boot.deployment_id.clone(),
                instance_id: boot.instance_id.clone(),
                instance_incarnation: boot.instance_incarnation.clone(),
                boot_id: boot.boot_id.clone(),
                authority_generation: boot.authority_generation,
                host_fence_id: fence.host_fence_id.clone(),
                host_fence_generation: fence.fence_generation,
                caller_id: self.config.caller_id.clone(),
                session_id: self.config.session_id.clone(),
                now_millis: self.recovery_now_millis(),
                ttl_seconds: self.config.recovery_ttl_seconds,
                renewal_interval_seconds: self.config.recovery_renewal_interval_seconds,
            };
            let Some(store) = self.recovery.as_mut() else {
                return (503, json_error("recovery_persistence_unavailable"));
            };
            let lease = match store.acquire_lease(request) {
                Ok(lease) => lease,
                Err(error) => return super::recovery_wire::recovery_store_error(error),
            };
            self.recovery_lease = Some(lease.clone());
            self.lease_active = false;
            return match self.install_host_lease(&boot, &fence, &lease, &Uuid::new_v4().to_string())
            {
                Ok(lease) => allocation_response(self, &lease),
                Err(error) => error.body(),
            };
        }
        self.lease_active = true;
        (
            200,
            json_bytes(&json!({
                "status": "allocated",
                "instance_id": self.config.instance_id,
                "caller_id": self.config.caller_id,
                "session_id": self.config.session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "transport": "attached-loopback"
            })),
        )
    }

    pub(super) fn release(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        let pending_revoke_retry = self.pending_host_revoke_matches(request);
        if !pending_revoke_retry && let Err(error) = self.check_lease(request) {
            return error;
        }
        self.allocation_cleanup_lease_id = None;
        if self.recovery.is_some() {
            let Some(lease) = self.recovery_lease.clone() else {
                return (409, json_error("lease_not_active"));
            };
            let correlation = request
                .headers
                .get("x-sts2-correlation-id")
                .map(String::as_str)
                .unwrap_or_default();
            if let Err(error) = self.revoke_host_lease(&lease.proof(), "shutdown", correlation) {
                return error.body();
            }
        }
        self.lease_active = false;
        self.lease_revoked = true;
        (
            200,
            json_bytes(&json!({
                "status": "released",
                "instance_id": self.config.instance_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch
            })),
        )
    }

    pub(super) fn relay_data(
        &mut self,
        request: &HttpRequest,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if body.len() > MAX_BODY_BYTES
            || (!body.is_empty() && serde_json::from_slice::<Value>(body).is_err())
        {
            return (400, json_error("runtime_body_invalid"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod(method, path, body, correlation) {
            Ok(response) if response.body.len() <= MAX_BODY_BYTES => {
                (response.status, response.body)
            }
            Ok(_) => (502, json_error("downstream_response_oversized")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    pub(super) fn check_lease(&mut self, request: &HttpRequest) -> Result<(), (u16, Vec<u8>)> {
        if self.recovery.is_some() {
            self.check_recovery_deadline();
            if !self.lease_active || self.lease_revoked || self.shutdown_requested {
                return Err((409, json_error("lease_not_active")));
            }
            let Some(lease) = self.recovery_lease.clone() else {
                return Err((409, json_error("lease_not_active")));
            };
            let now = self.recovery_now_millis();
            if let Some(store) = self.recovery.as_mut()
                && let Err(error) = store.validate_lease(&lease.proof(), now)
            {
                return Err(super::recovery_wire::recovery_store_error(error));
            }
            if !self
                .active_host_grant_matches(&lease)
                .map_err(super::recovery_wire::recovery_store_error)?
            {
                return Err((503, json_error("recovery_host_lease_required")));
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
            if expected
                .iter()
                .any(|(name, value)| request.headers.get(*name).map(String::as_str) != Some(*value))
            {
                return Err((409, json_error("lease_fence_rejected")));
            }
            let Some(correlation) = request.headers.get("x-sts2-correlation-id") else {
                return Err((400, json_error("correlation_required")));
            };
            if !safe_identity(correlation) {
                return Err((400, json_error("correlation_invalid")));
            }
            return Ok(());
        }
        if !self.lease_active {
            return Err((409, json_error("lease_not_active")));
        }
        let expected_epoch = self.config.lease_epoch.to_string();
        let expected = [
            ("x-sts2-instance-id", self.config.instance_id.as_str()),
            ("x-sts2-caller-id", self.config.caller_id.as_str()),
            ("x-sts2-session-id", self.config.session_id.as_str()),
            ("x-mcp-session-id", self.config.mcp_session_id.as_str()),
            ("x-sts2-lease-id", self.config.lease_id.as_str()),
            ("x-sts2-lease-epoch", expected_epoch.as_str()),
        ];
        if expected
            .iter()
            .any(|(name, value)| request.headers.get(*name).map(String::as_str) != Some(*value))
        {
            return Err((409, json_error("lease_fence_rejected")));
        }
        let Some(correlation) = request.headers.get("x-sts2-correlation-id") else {
            return Err((400, json_error("correlation_required")));
        };
        if !safe_identity(correlation) {
            return Err((400, json_error("correlation_invalid")));
        }
        Ok(())
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationRequest {
    instance_id: String,
    caller_id: String,
    session_id: String,
}
