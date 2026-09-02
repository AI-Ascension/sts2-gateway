// SPDX-License-Identifier: MIT

use crate::identity::{InstanceId, LeaseProof};
use crate::ports::{Clock, LeaseDecisionPort, ProcessPort};
use crate::{Gateway, GatewayError};

impl<C, P, R, T, F> Gateway<C, P, R, T, F>
where
    C: Clock,
    P: ProcessPort,
    F: LeaseDecisionPort,
{
    pub(crate) fn validate_fence(
        &mut self,
        target: InstanceId,
        proof: LeaseProof,
    ) -> Result<(), GatewayError> {
        let Some(record) = self.instances.get(&target) else {
            return Err(GatewayError::InstanceNotFound);
        };
        let decision =
            self.fence
                .check_fence(record.lease().copied(), target, proof, self.clock.now());
        if matches!(decision, Err(crate::FenceFailure::Expired)) {
            let _ = self.expire_instance(target);
        }
        decision.map_err(GatewayError::Fence)
    }
}
