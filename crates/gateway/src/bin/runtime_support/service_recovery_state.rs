// SPDX-License-Identifier: MIT

use std::time::Instant;

use serde_json::Value;
use sts2_gateway::{RecoveryBootContext, RecoveryBootState, RecoveryHostFence, RecoveryLeaseProof};

use super::RuntimeService;

impl RuntimeService {
    pub(super) fn current_boot_from_wire(&self, value: &Value) -> Option<RecoveryBootContext> {
        let current = self.recovery_boot.as_ref()?.clone();
        if !super::recovery_wire::boot_wire_shape(value)
            || value["deployment_id"].as_str() != Some(current.deployment_id.as_str())
            || value["instance_id"].as_str() != Some(current.instance_id.as_str())
            || value["instance_incarnation"].as_str() != Some(current.instance_incarnation.as_str())
            || value["boot_id"].as_str() != Some(current.boot_id.as_str())
            || value["authority_generation"].as_u64() != Some(current.authority_generation)
            || value["state"].as_str() != Some(super::recovery_wire::boot_state_name(current.state))
            || super::recovery_wire::parse_release(&value["release"])
                != Some(current.release.clone())
        {
            return None;
        }
        Some(current)
    }

    pub(super) fn current_fence_from_wire(&self, value: &Value) -> Option<RecoveryHostFence> {
        let current = self.recovery_fence.as_ref()?.clone();
        let wire = super::recovery_wire::parse_fence_wire(value)?;
        (wire.host_fence_id == current.host_fence_id
            && wire.deployment_id == current.deployment_id
            && wire.instance_id == current.instance_id
            && wire.instance_incarnation == current.instance_incarnation
            && wire.boot_id == current.boot_id
            && wire.authority_generation == current.authority_generation
            && wire.fence_generation == current.fence_generation)
            .then_some(current)
    }

    pub(super) fn lease_proof_from_wire(&mut self, value: &Value) -> Option<RecoveryLeaseProof> {
        self.check_recovery_deadline();
        let current = self.recovery_lease.as_ref()?.clone();
        let wire = super::recovery_wire::parse_lease_wire(value)?;
        if wire.deployment_id != current.deployment_id
            || wire.instance_id != current.instance_id
            || wire.instance_incarnation != current.instance_incarnation
            || wire.boot_id != current.boot_id
            || wire.authority_generation != current.authority_generation
            || wire.lease_id != current.lease_id
            || wire.lease_epoch != current.lease_epoch
            || wire.fence_token != current.fence_token
            || wire.ttl_seconds != current.ttl_seconds
            || wire.renewal_interval_seconds != current.renewal_interval_seconds
        {
            return None;
        }
        Some(current.proof())
    }

    pub(super) fn check_recovery_deadline(&mut self) {
        if self
            .recovery_lease_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            let now = self.recovery_now_millis();
            if let Some(store) = self.recovery.as_mut() {
                let _ = store.expire_leases(now);
            }
            self.recovery_lease = None;
            self.recovery_lease_deadline = None;
            self.lease_active = false;
        }
    }

    pub(super) fn recovery_ready(&mut self) -> bool {
        self.check_recovery_deadline();
        self.recovery
            .as_ref()
            .zip(self.recovery_boot.as_ref())
            .zip(self.recovery_fence.as_ref())
            .is_some_and(|((_, boot), _)| boot.state == RecoveryBootState::Ready)
    }

    pub(super) fn recovery_now_millis(&mut self) -> u64 {
        let wall = super::unix_millis();
        let elapsed = self
            .recovery_clock_started
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let monotonic = self.recovery_clock_wall_millis.saturating_add(elapsed);
        let now = wall.max(monotonic).max(self.recovery_last_now_millis);
        self.recovery_last_now_millis = now;
        now
    }
}
