// SPDX-License-Identifier: MIT

//! The legacy fixed-route table.
//!
//! Split out of `service_routes.rs` so the ordered pre-dispatch guards (which decide whether a
//! request belongs to a newer surface at all) stay readable next to the fixed table they fall
//! through to.

use super::*;

impl RuntimeService {
    /// Dispatches one request against the fixed method/path table.
    pub(super) fn dispatch_fixed_route(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/v1/recovery/bootstrap")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::Bootstrap,
                )
            }
            ("POST", "/v1/recovery/lease/acquire")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::LeaseAcquire,
                )
            }
            ("POST", "/v1/recovery/lease/renew")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::LeaseRenew,
                )
            }
            ("POST", "/v1/recovery/lease/revoke")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::LeaseRevoke,
                )
            }
            ("POST", "/v1/recovery/operation/intent")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::OperationIntent,
                )
            }
            ("POST", "/v1/recovery/operation/dispatch")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::OperationDispatch,
                )
            }
            ("POST", "/v1/recovery/operation/lookup")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::OperationLookup,
                )
            }
            ("POST", "/v1/recovery/operation/reconcile")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_route(
                    request,
                    super::super::recovery_frame::RecoveryKind::OperationReconcile,
                )
            }
            ("GET", path) if path == self.coop_synchronization_path() => {
                self.coop_synchronization(request)
            }
            ("POST", path) if path == self.coop_report_path() => self.coop_peer_report(request),
            ("POST", "/v1/recovery/host-fence")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.recovery_host_fence(request)
            }
            ("POST", path) if path == self.coop_receipt_query_path() => {
                self.coop_receipt_query(request)
            }
            ("GET", "/health/ready") if request.body.is_empty() => self.health(),
            ("GET", "/health/live") if request.body.is_empty() => self.health_live(),
            ("GET", "/health/progress") if request.body.is_empty() => self.health_progress(),
            ("POST", "/v1/sessions/allocate")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.allocate(&request.body)
            }
            ("GET", path) if path == self.state_path() && request.body.is_empty() => {
                self.relay_data(request, "GET", "/api/v1/runtime/state", &[])
            }
            ("POST", path) if path == self.action_path() && request.content_type_is_json() => {
                if request.body.is_empty() {
                    (400, json_error("action_body_required"))
                } else {
                    self.relay_data(request, "POST", "/api/v1/runtime/action", &request.body)
                }
            }
            ("POST", path)
                if path == self.runtime_v2_action_path() && request.content_type_is_json() =>
            {
                self.runtime_v2_action(request)
            }
            ("GET", path) if path == self.runtime_v2_state_path() => self.runtime_v2_state(request),
            ("GET", path) if path == self.runtime_v2_metrics_path() && request.body.is_empty() => {
                self.runtime_v2_metrics()
            }
            ("GET", path) if request.body.is_empty() => {
                let Some(operation_id) = self.runtime_v2_operation_id(path) else {
                    return (404, json_error("route_not_found"));
                };
                self.runtime_v2_reconcile(request, operation_id)
            }
            ("POST", path)
                if path == self.runtime_v2_shutdown_path() && request.body.is_empty() =>
            {
                self.runtime_v2_shutdown(request)
            }
            ("POST", path) if path == self.release_path() && request.body.is_empty() => {
                self.release(request)
            }
            _ => (404, json_error("route_not_found")),
        }
    }
}
