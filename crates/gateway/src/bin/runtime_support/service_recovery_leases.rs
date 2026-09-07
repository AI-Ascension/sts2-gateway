// SPDX-License-Identifier: MIT

impl RuntimeService {

    fn recovery_lease_acquire(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(boot) = self.current_boot_from_wire(&frame.payload()["boot"]) else {
            return (409, json_error("recovery_stale_boot"));
        };
        let Some(fence) = self.current_fence_from_wire(&frame.payload()["fence"]) else {
            return (409, json_error("recovery_stale_fence"));
        };
        let request = RecoveryLeaseRequest {
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
        self.recovery_lease_deadline =
            Some(Instant::now() + std::time::Duration::from_secs(lease.ttl_seconds));
        self.recovery_lease = Some(lease.clone());
        self.lease_active = true;
        self.lease_revoked = false;
        let body = json!({
            "result": response_result("LEASE_ACTIVE", false, None),
            "lease": super::recovery_wire::lease_value(&lease),
        });
        (
            200,
            response_frame(
                RecoveryKind::LeaseAcquire,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }

    fn recovery_lease_renew(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(proof) = self.lease_proof_from_wire(&frame.payload()["lease"]) else {
            return (409, json_error("recovery_stale_lease"));
        };
        let Some(sequence) = frame.payload()["renew_sequence"].as_u64() else {
            return (400, json_error("recovery_renew_sequence_invalid"));
        };
        let now = self.recovery_now_millis();
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        let lease = match store.renew_lease(&proof, sequence, now) {
            Ok(lease) => lease,
            Err(error) => return super::recovery_wire::recovery_store_error(error),
        };
        self.recovery_lease_deadline =
            Some(Instant::now() + std::time::Duration::from_secs(lease.ttl_seconds));
        self.recovery_lease = Some(lease.clone());
        let body = json!({
            "result": response_result("LEASE_RENEWED", false, None),
            "lease": super::recovery_wire::lease_value(&lease),
        });
        (
            200,
            response_frame(
                RecoveryKind::LeaseRenew,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }
}
