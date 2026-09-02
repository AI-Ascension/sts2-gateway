// SPDX-License-Identifier: MIT

use crate::identity::{InstanceId, LeaseProof, OperationId};
use crate::ports::{
    Clock, FixedRoute, LeaseDecisionPort, ProcessFault, ProcessHandle, ProcessPort, TransportPort,
    TransportRequest, TransportResponse,
};
use crate::{Gateway, GatewayError};

impl<C, P, R, T, F> Gateway<C, P, R, T, F>
where
    C: Clock,
    P: ProcessPort,
    T: TransportPort,
    F: LeaseDecisionPort,
{
    pub fn forward(
        &mut self,
        target: InstanceId,
        proof: LeaseProof,
        operation_id: OperationId,
        route: FixedRoute,
        body: Vec<u8>,
    ) -> Result<TransportResponse, GatewayError> {
        let actual = body.len();
        if actual > self.config.max_body_bytes() {
            return Err(GatewayError::BodyTooLarge {
                limit: self.config.max_body_bytes(),
                actual,
            });
        }
        let process = self.authorize_forward(target, proof)?;
        let request = TransportRequest::new(target, process, proof, operation_id, route, body);
        match self.transport.forward(request) {
            Ok(response) if response.body_len() <= self.config.max_response_bytes() => {
                if let Some(record) = self.instances.get_mut(&target) {
                    record.mark_ready();
                }
                Ok(response)
            }
            Ok(response) => {
                if let Some(record) = self.instances.get_mut(&target) {
                    record.mark_degraded();
                }
                Err(GatewayError::ResponseTooLarge {
                    limit: self.config.max_response_bytes(),
                    actual: response.body_len(),
                })
            }
            Err(fault) => {
                if let Some(record) = self.instances.get_mut(&target) {
                    record.mark_degraded();
                }
                Err(GatewayError::Transport(fault))
            }
        }
    }

    fn authorize_forward(
        &mut self,
        target: InstanceId,
        proof: LeaseProof,
    ) -> Result<ProcessHandle, GatewayError> {
        self.validate_fence(target, proof)?;
        let Some(record) = self.instances.get(&target) else {
            return Err(GatewayError::InstanceNotFound);
        };
        if !record.state().accepts_forwarding() {
            return Err(GatewayError::InvalidState(record.state()));
        }
        let Some(process) = record.process() else {
            if let Some(record) = self.instances.get_mut(&target) {
                record.mark_degraded();
            }
            return Err(GatewayError::ProcessInspection(ProcessFault::Unavailable));
        };
        if let Some(record) = self.instances.get_mut(&target) {
            record.mark_busy();
        }
        Ok(process)
    }
}
