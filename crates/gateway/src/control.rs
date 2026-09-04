// SPDX-License-Identifier: MIT

use crate::identity::{CallerId, InstanceId, Lease, LeaseEpoch, LeaseId, LeaseProof, SessionId};
use crate::lifecycle::{InstanceRecord, InstanceSnapshot, LifecycleState};
use crate::ports::{
    Clock, LaunchSpec, LeaseDecisionPort, ProcessFault, ProcessPort, ProcessState, Readiness,
    ReadinessPort, StopMode,
};
use crate::{Allocation, CleanupResult, CleanupStatus, Gateway, GatewayError};

impl<C, P, R, T, F> Gateway<C, P, R, T, F>
where
    C: Clock,
    P: ProcessPort,
    R: ReadinessPort,
    F: LeaseDecisionPort,
{
    pub fn allocate(
        &mut self,
        caller_id: CallerId,
        session_id: SessionId,
    ) -> Result<Allocation, GatewayError> {
        if !self.is_admitting {
            return Err(GatewayError::AdmissionClosed);
        }
        if self.instances.len() >= self.config.capacity() {
            return Err(GatewayError::CapacityExceeded);
        }

        let instance_id = self.next_instance()?;
        let lease_id = self.next_lease()?;
        let lease = Lease::new(
            instance_id,
            caller_id,
            session_id,
            lease_id,
            LeaseEpoch::new(1),
            self.clock
                .now()
                .saturating_add_millis(self.config.lease_duration_millis()),
        );
        self.instances.insert(
            instance_id,
            InstanceRecord::new(instance_id, caller_id, session_id, lease),
        );

        let process = match self.process.start(LaunchSpec::new(instance_id)) {
            Ok(process) => process,
            Err(fault) => {
                // A failed start transfers no process handle; its port owns partial-launch cleanup.
                // The caller never received this allocation, so do not retain an unreachable slot.
                let _ = self.instances.remove(&instance_id);
                return Err(GatewayError::ProcessStart(fault));
            }
        };

        let Some(record) = self.instances.get_mut(&instance_id) else {
            return Err(GatewayError::InstanceNotFound);
        };
        record.assign_process(process);
        Ok(Allocation { instance_id, lease })
    }

    pub fn status(&self, instance_id: InstanceId) -> Result<InstanceSnapshot, GatewayError> {
        self.instances
            .get(&instance_id)
            .map(InstanceRecord::snapshot)
            .ok_or(GatewayError::InstanceNotFound)
    }

    pub fn reconcile(&mut self, instance_id: InstanceId) -> Result<LifecycleState, GatewayError> {
        let Some(record) = self.instances.get(&instance_id) else {
            return Err(GatewayError::InstanceNotFound);
        };
        if record.state().is_terminal() {
            return Ok(record.state());
        }
        if self.lease_expired(instance_id) {
            return match self.expire_instance(instance_id) {
                CleanupStatus::Cleaned => Ok(LifecycleState::Expired),
                CleanupStatus::Failed(fault) => Err(GatewayError::ProcessStop(fault)),
            };
        }
        let Some(process) = record.process() else {
            if let Some(record) = self.instances.get_mut(&instance_id) {
                record.mark_failed();
            }
            return Err(GatewayError::ProcessInspection(ProcessFault::Unavailable));
        };

        let process_state = match self.process.inspect(process) {
            Ok(state) => state,
            Err(fault) => {
                if let Some(record) = self.instances.get_mut(&instance_id) {
                    record.mark_degraded();
                }
                return Err(GatewayError::ProcessInspection(fault));
            }
        };
        match process_state {
            ProcessState::Exited { .. } => {
                if let Some(record) = self.instances.get_mut(&instance_id) {
                    record.mark_failed();
                }
                Err(GatewayError::ProcessCrashed)
            }
            ProcessState::Running => match self.readiness.probe(instance_id, process) {
                Ok(Readiness::Starting) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_starting();
                    }
                    Ok(LifecycleState::Starting)
                }
                Ok(Readiness::Ready) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_ready();
                    }
                    Ok(LifecycleState::Ready)
                }
                Ok(Readiness::Degraded) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_degraded();
                    }
                    Ok(LifecycleState::Degraded)
                }
                Err(fault) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_degraded();
                    }
                    Err(GatewayError::Readiness(fault))
                }
            },
        }
    }

    pub fn renew(&mut self, proof: LeaseProof) -> Result<Lease, GatewayError> {
        self.validate_fence(proof.instance_id(), proof)?;
        let expires_at = self
            .clock
            .now()
            .saturating_add_millis(self.config.lease_duration_millis());
        let Some(record) = self.instances.get_mut(&proof.instance_id()) else {
            return Err(GatewayError::InstanceNotFound);
        };
        let Some(lease) = record.lease().copied() else {
            return Err(GatewayError::Fence(crate::FenceFailure::Missing));
        };
        let renewed = lease.renewed(expires_at);
        record.set_lease(renewed);
        Ok(renewed)
    }

    pub fn release(&mut self, proof: LeaseProof) -> Result<(), GatewayError> {
        self.validate_fence(proof.instance_id(), proof)?;
        let instance_id = proof.instance_id();
        let Some(record) = self.instances.get(&instance_id) else {
            return Err(GatewayError::InstanceNotFound);
        };
        if record.state().is_terminal() || record.state() == LifecycleState::Stopping {
            return Err(GatewayError::InvalidState(record.state()));
        }
        let process = record.process();
        if let Some(process) = process {
            if let Some(record) = self.instances.get_mut(&instance_id) {
                record.mark_stopping();
            }
            match self.process.stop(process, StopMode::Graceful) {
                Ok(()) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.clear_process();
                        record.mark_stopped();
                    }
                    Ok(())
                }
                Err(fault) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_failed();
                    }
                    Err(GatewayError::ProcessStop(fault))
                }
            }
        } else {
            if let Some(record) = self.instances.get_mut(&instance_id) {
                record.mark_stopped();
            }
            Ok(())
        }
    }

    pub fn cleanup(
        &mut self,
        instance_id: InstanceId,
        caller_id: CallerId,
        session_id: SessionId,
    ) -> Result<CleanupResult, GatewayError> {
        let Some(record) = self.instances.get(&instance_id) else {
            return Err(GatewayError::InstanceNotFound);
        };
        if record.caller_id() != caller_id || record.session_id() != session_id {
            return Err(GatewayError::IdentityMismatch);
        }
        if !record.state().is_terminal() {
            return Err(GatewayError::InvalidState(record.state()));
        }
        let process = record.process();
        if let Some(process) = process {
            match self.process.stop(process, StopMode::Force) {
                Ok(()) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.clear_process();
                    }
                }
                Err(fault) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_failed();
                    }
                    return Err(GatewayError::ProcessStop(fault));
                }
            }
        }
        let _ = self.instances.remove(&instance_id);
        Ok(CleanupResult::Removed)
    }

    fn next_instance(&mut self) -> Result<InstanceId, GatewayError> {
        let next = self
            .next_instance_id
            .checked_add(1)
            .ok_or(GatewayError::IdentityExhausted)?;
        let instance_id = InstanceId::new(self.next_instance_id);
        self.next_instance_id = next;
        Ok(instance_id)
    }

    fn next_lease(&mut self) -> Result<LeaseId, GatewayError> {
        let next = self
            .next_lease_id
            .checked_add(1)
            .ok_or(GatewayError::IdentityExhausted)?;
        let lease_id = LeaseId::new(self.next_lease_id);
        self.next_lease_id = next;
        Ok(lease_id)
    }
}
