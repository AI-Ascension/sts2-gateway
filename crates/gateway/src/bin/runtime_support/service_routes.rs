// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn handle_request(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Some(rejection) = request_rejection(
            request,
            &self.config.auth_policy,
            &self.config.instance_id,
            self.recovery.is_some(),
        ) {
            return rejection;
        }
        if request.method == "GET"
            && request.path
                == format!(
                    "/v1/instances/{}/checkpoint-reference",
                    self.config.instance_id
                )
        {
            return self.checkpoint_reference_request(request);
        }
        if let Some(route) =
            RuntimeV3GameplayRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.runtime_v3_request(request, route);
        }
        if let Some(route) =
            CoopNativeRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.coop_native_request(request, route);
        }
        if let Some(route) =
            RuntimeV4ExpertRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.runtime_v4_expert_request(request, route);
        }
        if let Some(route) = RuntimeV4ExpertRestActionRoute::parse(
            &request.method,
            &request.path,
            &self.config.instance_id,
        ) {
            return self.runtime_v4_expert_rest_action_request(request, route);
        }
        if let Some(route) =
            RuntimeMapRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.runtime_map_request(request, route);
        }
        if request.method == "POST"
            && request.path == self.seeded_run_start_path()
            && request.content_type_is_json()
        {
            return self.seeded_run_start(request);
        }
        if request.method == "GET"
            && request.body.is_empty()
            && let Some(operation_id) = self.seeded_run_operation_id(&request.path)
        {
            return self.seeded_run_reconcile(request, operation_id);
        }
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

    pub(super) fn runtime_v2_metrics(&self) -> (u16, Vec<u8>) {
        (
            200,
            json_bytes(
                &self
                    .metrics
                    .snapshot(&self.config.instance_id, self.config.queue_capacity),
            ),
        )
    }

    pub(super) fn runtime_v2_shutdown(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        let pending_revoke_retry = self.pending_host_revoke_matches(request);
        if !pending_revoke_retry && let Err(error) = self.check_lease(request) {
            return error;
        }
        self.allocation_cleanup_lease_id = None;
        self.shutdown_requested = true;
        if self.recovery.is_some() {
            let Some(lease) = self.recovery_lease.clone() else {
                return (409, json_error("lease_not_active"));
            };
            let correlation = request
                .headers
                .get("x-sts2-correlation-id")
                .map(String::as_str)
                .unwrap_or_default();
            if let Err(error) = self.revoke_host_lease(&lease.proof(), "shutdown", correlation) {
                return error.body();
            }
        }
        self.lease_active = false;
        self.lease_revoked = true;
        self.shutdown_requested = true;
        self.metrics.request_shutdown();
        (
            202,
            json_bytes(&json!({
                "status": "shutdown_requested",
                "instance_id": self.config.instance_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch
            })),
        )
    }

    pub(super) fn state_path(&self) -> String {
        format!("/v1/instances/{}/state", self.config.instance_id)
    }

    pub(super) fn action_path(&self) -> String {
        format!("/v1/instances/{}/action", self.config.instance_id)
    }

    pub(super) fn release_path(&self) -> String {
        format!("/v1/instances/{}/release", self.config.instance_id)
    }

    pub(super) fn runtime_v2_action_path(&self) -> String {
        format!("/v2/instances/{}/action", self.config.instance_id)
    }

    pub(super) fn runtime_v2_state_path(&self) -> String {
        format!("/v2/instances/{}/state", self.config.instance_id)
    }

    pub(super) fn runtime_v2_metrics_path(&self) -> String {
        format!("/v2/instances/{}/metrics", self.config.instance_id)
    }

    pub(super) fn runtime_v2_shutdown_path(&self) -> String {
        format!("/v2/instances/{}/shutdown", self.config.instance_id)
    }

    pub(super) fn runtime_v2_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("/v2/instances/{}/operations/", self.config.instance_id);
        path.strip_prefix(&prefix)
            .filter(|operation_id| !operation_id.is_empty() && !operation_id.contains('/'))
    }

    pub(super) fn health(&mut self) -> (u16, Vec<u8>) {
        if self.recovery.is_some() && !self.recovery_ready() {
            return (503, json_error("recovery_host_fence_required"));
        }
        match self.forward_mod("GET", "/health/ready", &[], None) {
            Ok(response) if response.status == 200 => (
                200,
                json_bytes(&json!({
                    "status": "ready",
                    "instance_id": self.config.instance_id,
                    "downstream": "ready"
                })),
            ),
            Ok(_) => (503, json_error("downstream_not_ready")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    pub(super) fn health_live(&self) -> (u16, Vec<u8>) {
        (
            200,
            json_bytes(&json!({
                "status": "live",
                "instance_id": self.config.instance_id,
            })),
        )
    }

    pub(super) fn health_progress(&mut self) -> (u16, Vec<u8>) {
        let mut body = self
            .metrics
            .snapshot(&self.config.instance_id, self.config.queue_capacity);
        if let Some(store) = self.recovery.as_ref() {
            match store.unresolved_count() {
                Ok(count) => body["recovery_unresolved_operations"] = count.into(),
                Err(_) => return (503, json_error("recovery_persistence_unavailable")),
            }
            body["recovery_ready"] = self.recovery_ready().into();
        } else {
            body["recovery_ready"] = true.into();
        }
        (200, json_bytes(&body))
    }
}
