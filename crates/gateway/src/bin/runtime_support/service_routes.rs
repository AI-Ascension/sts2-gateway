// SPDX-License-Identifier: MIT

use super::super::save_profile::RuntimeSaveProfileRoute;
use super::*;

#[path = "service_exact_restore.rs"]
mod exact_restore;
#[path = "service_recovery_owner.rs"]
mod recovery_owner;

impl RuntimeService {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn handle_request(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        let cancellation = RequestCancellation::new();
        self.handle_request_with_cancellation(request, &cancellation)
    }

    pub(super) fn handle_request_with_cancellation(
        &mut self,
        request: &HttpRequest,
        cancellation: &RequestCancellation,
    ) -> (u16, Vec<u8>) {
        if let Some(rejection) = request_rejection(
            request,
            &self.config.auth_policy,
            &self.config.instance_id,
            self.recovery.is_some(),
        ) {
            return rejection;
        }
        if let Some(response) = super::checkpoint_reference::dispatch(self, request) {
            return response;
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
        if request.method == "GET"
            && request.path == super::negotiated_capabilities::path(&self.config.instance_id)
        {
            return self.negotiated_capabilities(request);
        }
        if let Some(route) =
            GameInformationRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.game_information_request(request, route, cancellation);
        }
        if let Some(route) =
            RuntimeSaveProfileRoute::parse(&request.method, &request.path, &self.config.instance_id)
        {
            return self.save_profile_request(request, route);
        }
        if let Some(route) = super::process_lifecycle_wire::LifecycleRoute::parse(
            &request.method,
            &request.path,
            &self.config.instance_id,
        ) {
            return self.process_lifecycle_request(request, route);
        }
        if let Some(route) = exact_restore::Route::parse(&request.method, &request.path) {
            return exact_restore::handle(self, request, route);
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
        if let Some(response) = recovery_owner::dispatch(self, request) {
            return response;
        }
        self.dispatch_fixed_route(request)
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

#[cfg(test)]
#[path = "service_recovery_owner_tests.rs"]
mod recovery_owner_tests;
