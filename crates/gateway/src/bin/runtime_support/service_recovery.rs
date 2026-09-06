// SPDX-License-Identifier: MIT

use super::super::recovery_control::RecoveryControlTransportFault;
use super::{HttpRequest, RuntimeService, json_error};

impl RuntimeService {
    /// Bridges the authenticated gateway control route to the mod's fixed recovery endpoint.
    ///
    /// This route deliberately does not call `check_lease`: the recovery frame carries the new
    /// boot/fence identity and is the bootstrap path used before a gameplay lease exists.
    pub(super) fn recovery_host_fence(&self, request: &HttpRequest) -> (u16, Vec<u8>) {
        let forwarder = super::super::recovery_control::HttpRecoveryControlForwarder::new(
            &self.config.mod_address,
            &self.config.mod_token,
        );
        match forwarder.forward_host_fence(&request.body) {
            Ok(response) => (response.status, response.body),
            Err(error) => recovery_control_error(error),
        }
    }
}

fn recovery_control_error(error: RecoveryControlTransportFault) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RecoveryControlTransportFault::InvalidFrame => (400, "recovery_host_fence_frame_invalid"),
        RecoveryControlTransportFault::RequestOversized => (413, "recovery_frame_oversized"),
        RecoveryControlTransportFault::InvalidConfiguration => {
            (500, "recovery_control_configuration_invalid")
        }
        RecoveryControlTransportFault::UnavailableBeforeWrite => (503, "recovery_host_unavailable"),
        RecoveryControlTransportFault::DisconnectedAfterWrite
        | RecoveryControlTransportFault::TimeoutAfterWrite => {
            (503, "recovery_host_fence_outcome_unknown")
        }
        RecoveryControlTransportFault::MalformedResponse => {
            (502, "recovery_host_fence_response_invalid")
        }
    };
    (status, json_error(code))
}
