// SPDX-License-Identifier: MIT

impl RuntimeService {

    fn recovery_lease_acquire(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(boot) = self.current_boot_from_wire(&frame.payload()["boot"]) else {
            return (409, json_error("recovery_stale_boot"));
        };
        let Some(fence) = self.current_fence_from_wire(&frame.payload()["fence"]) else {
            return (409, json_error("recovery_stale_fence"));
        };
        // A lost install acknowledgment is retried with the same in-memory
        // lease and installation identity.  A fresh process has no token
        // material and therefore cannot take this branch.
        if let Some(pending) = self.recovery_lease.clone()
            && !self.lease_active
            && pending.boot_id == boot.boot_id
            && self
                .recovery
                .as_ref()
                .and_then(|store| store.host_lease_binding(&pending.lease_id).ok().flatten())
                .is_some_and(|binding| {
                    binding.state == sts2_gateway::RecoveryHostLeaseState::PendingHostInstall
                })
        {
            match self.install_host_lease(&boot, &fence, &pending, frame.correlation()) {
                Ok(lease) => return lease_acquire_response(self, frame, &lease),
                Err(error) => return error.body(),
            }
        }
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
        self.recovery_lease = Some(lease.clone());
        self.lease_active = false;
        match self.install_host_lease(&boot, &fence, &lease, frame.correlation()) {
            Ok(lease) => lease_acquire_response(self, frame, &lease),
            Err(error) => error.body(),
        }
    }

    fn recovery_lease_renew(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(proof) = self.lease_proof_from_wire(&frame.payload()["lease"]) else {
            return (409, json_error("recovery_stale_lease"));
        };
        let Some(sequence) = frame.payload()["renew_sequence"].as_u64() else {
            return (400, json_error("recovery_renew_sequence_invalid"));
        };
        if self.recovery.is_none() {
            return (503, json_error("recovery_persistence_unavailable"));
        }
        match self.renew_host_lease(&proof, sequence, frame.correlation()) {
            Ok(lease) => {
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
            Err(error) => error.body(),
        }
    }
}

fn lease_acquire_response(
    service: &RuntimeService,
    frame: &RecoveryFrame,
    lease: &sts2_gateway::RecoveryLease,
) -> (u16, Vec<u8>) {
    let body = json!({
        "result": response_result("LEASE_ACTIVE", false, None),
        "lease": super::recovery_wire::lease_value(lease),
    });
    (
        200,
        response_frame(
            RecoveryKind::LeaseAcquire,
            frame.correlation(),
            &service.config.caller_id,
            body,
        ),
    )
}
