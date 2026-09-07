// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::RecoveryLeaseRequest;

use super::super::recovery_frame::{
    RecoveryFrame, RecoveryFrameError, RecoveryKind, parse_response_frame, request_frame,
    response_frame, response_result,
};
use super::{HttpRequest, RuntimeService, json_error};

impl RuntimeService {
    /// Handles the legacy host-fence bridge when no durable recovery store is
    /// configured, and the authenticated sideband route otherwise.
    pub(super) fn recovery_host_fence(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if self.recovery.is_none() {
            let forwarder = super::super::recovery_control::HttpRecoveryControlForwarder::new(
                &self.config.mod_address,
                &self.config.mod_token,
            );
            return match forwarder.forward_host_fence(&request.body) {
                Ok(response) => (response.status, response.body),
                Err(error) => super::recovery_wire::recovery_control_error(error),
            };
        }
        self.recovery_route(request, RecoveryKind::HostFence)
    }

    pub(super) fn recovery_route(
        &mut self,
        request: &HttpRequest,
        kind: RecoveryKind,
    ) -> (u16, Vec<u8>) {
        if self.recovery.is_some()
            && request
                .headers
                .get("x-sts2-recovery-capability")
                .map(String::as_str)
                != Some(kind.capability())
        {
            return (403, json_error("recovery_capability_forbidden"));
        }
        if !request.content_type_is_json() || request.body.is_empty() {
            return (400, json_error("recovery_frame_required"));
        }
        let frame = match RecoveryFrame::parse(&request.body, kind) {
            Ok(frame) => frame,
            Err(RecoveryFrameError::Oversized) => {
                return (413, json_error("recovery_frame_oversized"));
            }
            Err(RecoveryFrameError::Invalid) => {
                return (400, json_error("recovery_frame_invalid"));
            }
        };
        if frame.value["actor"]["principal_id"].as_str() != Some(self.config.caller_id.as_str())
            || frame.value["auth"]["principal_id"].as_str() != Some(self.config.caller_id.as_str())
        {
            return (403, json_error("recovery_principal_forbidden"));
        }
        match kind {
            RecoveryKind::Bootstrap => self.recovery_bootstrap(&frame),
            RecoveryKind::HostFence => self.recovery_host_fence_frame(&frame),
            RecoveryKind::LeaseAcquire => self.recovery_lease_acquire(&frame),
            RecoveryKind::LeaseRenew => self.recovery_lease_renew(&frame),
            RecoveryKind::LeaseRevoke => self.recovery_lease_revoke(&frame),
            RecoveryKind::OperationIntent => self.recovery_ops_intent(&frame),
            RecoveryKind::OperationDispatch => self.recovery_ops_dispatch(&frame),
            RecoveryKind::OperationLookup => self.recovery_ops_lookup(&frame),
            RecoveryKind::OperationReconcile => self.recovery_ops_reconcile(&frame),
        }
    }

    fn recovery_bootstrap(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let payload = frame.payload();
        let (Some(deployment_id), Some(instance_id), Some(incarnation)) = (
            payload["deployment_id"].as_str(),
            payload["instance_id"].as_str(),
            payload["instance_incarnation"].as_str(),
        ) else {
            return (400, json_error("recovery_bootstrap_invalid"));
        };
        if !super::recovery_wire::valid_uuid(deployment_id)
            || !super::recovery_wire::valid_uuid(instance_id)
            || !super::recovery_wire::valid_uuid_v4(incarnation)
        {
            return (400, json_error("recovery_bootstrap_identity_invalid"));
        }
        let Some(release) = super::recovery_wire::parse_release(&payload["release"]) else {
            return (400, json_error("recovery_bootstrap_release_invalid"));
        };
        let Some(policy) = super::recovery_wire::exact_object(
            &payload["lease_policy"],
            &["ttl_seconds", "renewal_interval_seconds"],
        ) else {
            return (400, json_error("recovery_bootstrap_policy_invalid"));
        };
        let (Some(ttl), Some(renewal)) = (
            policy["ttl_seconds"].as_u64(),
            policy["renewal_interval_seconds"].as_u64(),
        ) else {
            return (400, json_error("recovery_bootstrap_policy_invalid"));
        };
        if !(5..=300).contains(&ttl) || renewal == 0 || renewal >= ttl {
            return (400, json_error("recovery_bootstrap_policy_invalid"));
        }
        if self.config.recovery_deployment_id.as_deref() != Some(deployment_id)
            || self.config.instance_id != instance_id
            || release != self.config.recovery_release
            || ttl != self.config.recovery_ttl_seconds
            || renewal != self.config.recovery_renewal_interval_seconds
        {
            return (409, json_error("recovery_bootstrap_context_mismatch"));
        }
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let boot = match store.start_boot_with_incarnation(
            deployment_id,
            instance_id,
            incarnation,
            release,
            now,
        ) {
            Ok(boot) => boot,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        self.recovery_boot = Some(boot.clone());
        self.recovery_fence = None;
        self.recovery_lease = None;
        self.recovery_lease_deadline = None;
        self.recovery_host_grant = None;
        self.lease_active = false;
        self.lease_revoked = false;
        let body = json!({
            "result": response_result("BOOT_AUTHORITY_CREATED", false, None),
            "boot": super::recovery_wire::boot_value(&boot),
            "fence": Value::Null,
        });
        (
            200,
            response_frame(
                RecoveryKind::Bootstrap,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }

    fn recovery_host_fence_frame(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(boot) = self.current_boot_from_wire(&frame.payload()["boot"]) else {
            return (409, json_error("recovery_stale_boot"));
        };
        let Some(auth_proof) = self.recovery_host_fence_proof(&boot) else {
            return (503, json_error("recovery_auth_unavailable"));
        };
        // The caller's frame is authenticated at the gateway boundary, but its
        // proof is not valid on the gateway-to-host hop. Rebuild a fresh closed
        // frame with the host-fence proof derived from the local bootstrap secret.
        let host_frame = request_frame(
            RecoveryKind::HostFence,
            &self.config.caller_id,
            frame.correlation(),
            Some(&auth_proof),
            json!({"boot": super::recovery_wire::boot_value(&boot)}),
        );
        let forwarder = super::super::recovery_control::HttpRecoveryControlForwarder::new(
            &self.config.mod_address,
            &self.config.mod_token,
        );
        let response = match forwarder.forward_host_fence(&host_frame) {
            Ok(response) => response,
            Err(error) => return super::recovery_wire::recovery_control_error(error),
        };
        if response.status != 200 {
            return (response.status, response.body);
        }
        let value = match parse_response_frame(&response.body, RecoveryKind::HostFence) {
            Ok(value) => value,
            Err(RecoveryFrameError::Oversized) => {
                return (503, json_error("recovery_host_fence_outcome_unknown"));
            }
            Err(RecoveryFrameError::Invalid) => {
                return (503, json_error("recovery_host_fence_outcome_unknown"));
            }
        };
        if value["correlation_id"] != frame.value["correlation_id"] {
            return (409, json_error("recovery_correlation_mismatch"));
        }
        if value["payload"]["result"]["status"].as_str() != Some("FENCE_ACCEPTED") {
            return (409, json_error("recovery_host_fence_rejected"));
        }
        let Some(host_fence) = super::recovery_wire::parse_fence_wire(&value["payload"]["fence"])
        else {
            return (502, json_error("recovery_host_fence_response_invalid"));
        };
        if !super::recovery_wire::same_boot_fence(&host_fence, &boot) {
            return (409, json_error("recovery_host_fence_context_mismatch"));
        }
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let (fence, authoritative_boot) = {
            let fence = match store.complete_host_fence_from_host(
                &boot,
                &host_fence.host_fence_id,
                host_fence.fence_generation,
                now,
            ) {
                Ok(fence) => fence,
                Err(error) => return super::recovery_wire::recovery_store_error(error),
            };
            let authoritative_boot = match store.current_boot() {
                Ok(boot) => boot,
                Err(error) => return super::recovery_wire::recovery_store_error(error),
            };
            (fence, authoritative_boot)
        };
        // The durable transition is authoritative. Keep health/readiness and all
        // subsequent lease checks on the same READY boot that was committed.
        self.recovery_boot = Some(authoritative_boot);
        self.recovery_fence = Some(fence.clone());
        let body = json!({
            "result": response_result("FENCE_ACCEPTED", false, None),
            "fence": super::recovery_wire::fence_value(&fence),
        });
        (
            200,
            response_frame(
                RecoveryKind::HostFence,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }
}
