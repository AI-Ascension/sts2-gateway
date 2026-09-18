// SPDX-License-Identifier: MIT

//! Request handling for the fixed process-lifecycle route surface.
//!
//! Split from `service_process_lifecycle.rs` so the composition state and the HTTP surface stay
//! separately readable. Every identity this module uses comes from configuration or from the
//! request path; the body supplies only an operation id, an authority epoch, and one closed
//! action.

use sts2_gateway::{AuthorityEpoch, OperationId};

use super::super::http::HttpRequest;
use super::RuntimeService;
use super::process_lifecycle_wire::{LifecycleRoute, LifecycleSubmission, lifecycle_error};
use super::service_process_lifecycle::{NO_LEASE_EXPIRY, ProcessLifecycleRuntime};
use super::support::{json_bytes, json_error};

impl RuntimeService {
    /// Serves one fixed process-lifecycle route.
    ///
    /// Capability is readable whenever the deployment is configured, so an operator can see the
    /// validated profile set and why effects are refused. Every effect-bearing route requires the
    /// composed coordinator and otherwise refuses before any store or process call.
    pub(super) fn process_lifecycle_request(
        &mut self,
        request: &HttpRequest,
        route: LifecycleRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route == LifecycleRoute::Capability {
            return self.process_lifecycle_capability();
        }
        if !self.process_lifecycle.is_ready() {
            return self.process_lifecycle_unavailable();
        }
        match route {
            LifecycleRoute::Capability => self.process_lifecycle_capability(),
            LifecycleRoute::Lookup => self.process_lifecycle_lookup(request, route),
            LifecycleRoute::Submit => self.process_lifecycle_submit(request),
        }
    }

    /// Distinguishes "not configured" from "configured but no reviewed adapter is installed".
    fn process_lifecycle_unavailable(&self) -> (u16, Vec<u8>) {
        let code = match self.process_lifecycle {
            ProcessLifecycleRuntime::Unconfigured => "process_lifecycle_unconfigured",
            ProcessLifecycleRuntime::Configured { .. } => "process_lifecycle_adapter_absent",
            ProcessLifecycleRuntime::Ready { .. } => "process_lifecycle_unavailable",
        };
        (503, json_error(code))
    }

    /// The configured numeric instance identity every lifecycle operation is scoped to.
    pub(super) fn lifecycle_instance(&self) -> sts2_gateway::InstanceId {
        super::service_process_lifecycle_identity::instance_id(&self.config.instance_id)
    }

    /// The lease proof built from configured identities, never from the request body.
    pub(super) fn lifecycle_proof(&self) -> sts2_gateway::LeaseProof {
        super::service_process_lifecycle_identity::lease_proof(
            &self.config.instance_id,
            &self.config.caller_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    fn process_lifecycle_capability(&self) -> (u16, Vec<u8>) {
        let instance = self.lifecycle_instance();
        let Some(profiles) = self.process_lifecycle.profiles() else {
            return (503, json_error("process_lifecycle_unconfigured"));
        };
        let ready = self.process_lifecycle.is_ready();
        let authority_epoch = self
            .process_lifecycle
            .authority_epoch(instance)
            .map_or(serde_json::Value::Null, |epoch| epoch.value().into());
        let body = serde_json::json!({
            "contract": super::process_lifecycle_wire::LIFECYCLE_CONTRACT,
            // `available` reports whether effects are possible, not merely whether profiles exist.
            "available": ready,
            "profiles": profiles,
            "authority_epoch": authority_epoch,
            "instance_id": instance.value(),
            "unavailable_reason": (!ready).then_some("process_adapter_absent"),
        });
        (200, json_bytes(&body))
    }

    fn process_lifecycle_lookup(
        &mut self,
        request: &HttpRequest,
        route: LifecycleRoute,
    ) -> (u16, Vec<u8>) {
        let Some(operation_id) = route.operation_id(&request.path, &self.config.instance_id) else {
            return (404, json_error("route_not_found"));
        };
        let instance = self.lifecycle_instance();
        let Some(lifecycle) = self.process_lifecycle.lifecycle_mut() else {
            return self.process_lifecycle_unavailable();
        };
        match lifecycle.operation(instance, OperationId::new(operation_id)) {
            Some(operation) => (
                200,
                json_bytes(&serde_json::json!({
                    "contract": super::process_lifecycle_wire::LIFECYCLE_CONTRACT,
                    "operation_id": operation.operation_id().value(),
                    "instance_id": operation.instance_id().value(),
                    "sequence": operation.sequence(),
                    "state": operation.state(),
                    "action": operation.action(),
                    "process": operation.process(),
                    "authority_epoch": operation.authority_epoch().value(),
                    "request_epoch": operation.request_epoch().value(),
                    "failure": operation.failure(),
                })),
            ),
            None => (404, json_error("process_lifecycle_operation_not_found")),
        }
    }

    fn process_lifecycle_submit(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        let submission = match LifecycleSubmission::parse(&request.body) {
            Ok(submission) => submission,
            Err(error) => return error,
        };
        let instance = self.lifecycle_instance();
        let action = match submission.resolve_action(instance) {
            Ok(action) => action,
            Err(error) => return error,
        };
        let proof = self.lifecycle_proof();
        let Some(lifecycle) = self.process_lifecycle.lifecycle_mut() else {
            return self.process_lifecycle_unavailable();
        };
        if let Err(error) = lifecycle.bind_attached_lease(proof, NO_LEASE_EXPIRY) {
            return lifecycle_error(&error);
        }
        let operation_id = OperationId::new(submission.operation_id());
        let epoch = AuthorityEpoch::new(submission.authority_epoch());
        let request = match &action {
            sts2_gateway::LifecycleAction::LaunchNew { profile_id } => {
                sts2_gateway::LifecycleRequest::launch_new(operation_id, proof, epoch, *profile_id)
            }
            sts2_gateway::LifecycleAction::Restart { profile_id } => {
                sts2_gateway::LifecycleRequest::restart(operation_id, proof, epoch, *profile_id)
            }
            sts2_gateway::LifecycleAction::Stop { mode } => {
                sts2_gateway::LifecycleRequest::stop(operation_id, proof, epoch, *mode)
            }
            sts2_gateway::LifecycleAction::AttachExisting { identity } => {
                sts2_gateway::LifecycleRequest::attach_existing(
                    operation_id,
                    proof,
                    epoch,
                    identity.clone(),
                )
            }
        };
        match lifecycle.apply(request) {
            Ok(response) => (
                200,
                json_bytes(&serde_json::json!({
                    "contract": super::process_lifecycle_wire::LIFECYCLE_CONTRACT,
                    "operation_id": response.operation_id().value(),
                    "instance_id": response.instance_id().value(),
                    "state": response.state(),
                    "operation_state": response.operation_state(),
                    "process": response.process(),
                    "authority_epoch": response.authority_epoch().value(),
                    "failure": response.failure(),
                })),
            ),
            Err(error) => lifecycle_error(&error),
        }
    }
}
