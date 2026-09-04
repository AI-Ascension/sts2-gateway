// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn runtime_v3_action(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if let Ok(body) = serde_json::from_slice::<Value>(&request.body)
            && body
                .get("operation_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !safe_operation_id(id))
        {
            return (400, json_error("runtime_v3_operation_invalid"));
        }
        self.runtime_v3.action(
            request,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    pub(super) fn runtime_v3_state(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.runtime_v3.state(
            request,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    pub(super) fn runtime_v3_operation(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_operation_id(operation_id) {
            return (400, json_error("runtime_v3_operation_invalid"));
        }
        self.runtime_v3.operation(
            request,
            operation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }
}
