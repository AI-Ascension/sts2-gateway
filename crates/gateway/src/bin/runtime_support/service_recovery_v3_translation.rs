// SPDX-License-Identifier: MIT

impl RuntimeService {
    pub(super) fn runtime_v3_recovery_dispatch(
        &mut self,
        request: &super::HttpRequest,
        route: super::RuntimeV3GameplayRoute,
        envelope: Value,
    ) -> (u16, Vec<u8>) {
        let Some(proof) = self.recovery_lease.as_ref().map(|lease| lease.proof()) else {
            return self.recovery_v3_dispatch(request, route, envelope);
        };
        let raw = self.recovery_v3_dispatch(request, route, envelope.clone());
        let Ok(host) = super::super::strict_json::parse(&raw.1) else {
            return raw;
        };
        if host["kind"] != "operation_dispatch_response" {
            return raw;
        }
        self.runtime_v3_recovery_response(&proof, &envelope, raw)
    }

    fn runtime_v3_recovery_response(
        &mut self,
        proof: &RecoveryLeaseProof,
        request: &Value,
        operation_response: (u16, Vec<u8>),
    ) -> (u16, Vec<u8>) {
        let (host_status, body) = operation_response;
        let Some(host) = super::super::strict_json::parse(&body).ok() else {
            return runtime_v3_unknown_response(self, request, "recovery_response_invalid");
        };
        let Some(operation) = host["payload"]["operation"].as_object() else {
            return runtime_v3_unknown_response(self, request, "recovery_operation_missing");
        };
        if operation["operation_id"] != request["operation_id"] {
            return runtime_v3_unknown_response(self, request, "recovery_operation_mismatch");
        }
        let status = host["payload"]["result"]["status"]
            .as_str()
            .unwrap_or_default();
        if !matches!(status, "SETTLED" | "REJECTED") {
            let error_code = match status {
                "ACCEPTED" => "recovery_operation_pending",
                "UNKNOWN" => operation["uncertainty_reason"]
                    .as_str()
                    .unwrap_or("recovery_operation_unknown"),
                _ => "settlement_unproven",
            };
            return runtime_v3_unknown_response(self, request, error_code);
        }
        let Some(state) = self.runtime_v3_post_recovery_state(proof, request) else {
            return runtime_v3_unknown_response(self, request, "settlement_observation_unavailable");
        };
        let Some(state_id) = state["state_id"].as_str() else {
            return runtime_v3_unknown_response(self, request, "settlement_observation_invalid");
        };
        let Some(generation) = state["generation"].as_u64() else {
            return runtime_v3_unknown_response(self, request, "settlement_observation_invalid");
        };
        let settled = status == "SETTLED";
        if settled
            && (operation["witness"]["state_id"].as_str() != Some(state_id)
                || operation["witness"]["generation"].as_u64() != Some(generation))
        {
            return runtime_v3_unknown_response(self, request, "settlement_witness_mismatch");
        }
        if settled && generation <= request["generation"].as_u64().unwrap_or(u64::MAX) {
            return runtime_v3_unknown_response(self, request, "settlement_generation_invalid");
        }
        let mut response = state;
        response["kind"] = json!("dispatch_action_response");
        response["operation_id"] = request["operation_id"].clone();
        response["status"] = json!(if settled { "settled" } else { "rejected" });
        // The canonical schema requires a non-null `error_code` for a rejected
        // outcome. The host refusal is a typed terminal rejection, not an
        // uncertainty, so it must not degrade to the unknown fallback.
        response["error_code"] = if settled {
            Value::Null
        } else {
            json!("recovery_operation_rejected")
        };
        response["transition"] = if settled {
            json!({
                "from_generation": request["generation"],
                "to_generation": response["generation"],
                "state_id": response["state_id"],
                "effect_kind": "recovery_operation_settled",
            })
        } else {
            Value::Null
        };
        let encoded = match serde_json::to_vec(&response) {
            Ok(encoded) => encoded,
            Err(_) => return runtime_v3_unknown_response(self, request, "settlement_response_invalid"),
        };
        if self
            .runtime_v3
            .validate_response(
                super::RuntimeV3GameplayRoute::DispatchAction,
                request,
                &encoded,
            )
            .is_err()
        {
            return runtime_v3_unknown_response(self, request, "settlement_response_invalid");
        }
        (if settled { 200 } else { host_status }, encoded)
    }

    fn runtime_v3_post_recovery_state(
        &mut self,
        proof: &RecoveryLeaseProof,
        request: &Value,
    ) -> Option<Value> {
        self.check_recovery_deadline();
        if !self.recovery_authority_current(proof) {
            return None;
        }
        let lease = self.recovery_lease.clone()?;
        let state_request = runtime_v3_state_request(request);
        let bytes = serde_json::to_vec(&state_request).ok()?;
        let before_lease = self.recovery_lease.clone()?;
        let before_fence = self.recovery_fence.clone()?;
        let response = self
            .forward_mod(
                "GET",
                super::RuntimeV3GameplayRoute::State.downstream_path(),
                &bytes,
                state_request["correlation_id"].as_str(),
            )
            .ok()?;
        self.check_recovery_deadline();
        if self.recovery_lease != Some(before_lease)
            || self.recovery_fence != Some(before_fence)
            || !self.recovery_authority_current(proof)
        {
            return None;
        }
        if response.status != 200
            || self
                .runtime_v3
                .validate_response(
                    super::RuntimeV3GameplayRoute::State,
                    &state_request,
                    &response.body,
                )
                .is_err()
        {
            return None;
        }
        let state = super::super::strict_json::parse(&response.body).ok()?;
        self.observe_recovery_catalog(
            &lease,
            super::RuntimeV3GameplayRoute::State,
            200,
            &response.body,
        )
        .then_some(state)
    }

    fn recovery_authority_current(&mut self, proof: &RecoveryLeaseProof) -> bool {
        self.check_recovery_deadline();
        let Some(lease) = self
            .recovery_lease
            .as_ref()
            .filter(|lease| lease.proof() == *proof)
            .cloned()
        else {
            return false;
        };
        let fence_matches = self.recovery_fence.as_ref().is_some_and(|fence| {
            fence.deployment_id == proof.deployment_id
                && fence.instance_id == proof.instance_id
                && fence.instance_incarnation == proof.instance_incarnation
                && fence.boot_id == proof.boot_id
                && fence.authority_generation == proof.authority_generation
        });
        let boot_matches = self.recovery_boot.as_ref().is_some_and(|boot| {
            boot.state == sts2_gateway::RecoveryBootState::Ready
                && boot.deployment_id == proof.deployment_id
                && boot.instance_id == proof.instance_id
                && boot.instance_incarnation == proof.instance_incarnation
                && boot.boot_id == proof.boot_id
                && boot.authority_generation == proof.authority_generation
        });
        let grant_matches = self.active_host_grant_matches(&lease).is_ok_and(|ready| ready);
        self.lease_active && !self.lease_revoked && !self.shutdown_requested && fence_matches && boot_matches && grant_matches
    }
}

fn runtime_v3_state_request(request: &Value) -> Value {
    let mut state = request.clone();
    state["kind"] = json!("state_request");
    for field in [
        "state_id",
        "operation_id",
        "observation",
        "legal_actions",
        "action",
        "status",
        "transition",
        "error_code",
        "wait_for_millis",
        "wait_outcome",
        "recovery",
    ] {
        state[field] = Value::Null;
    }
    state
}

fn runtime_v3_unknown_response(
    service: &RuntimeService,
    request: &Value,
    error_code: &str,
) -> (u16, Vec<u8>) {
    let mut response = runtime_v3_state_request(request);
    response["kind"] = json!("dispatch_action_response");
    response["state_id"] = Value::Null;
    response["operation_id"] = request["operation_id"].clone();
    response["status"] = json!("unknown");
    response["error_code"] = json!(error_code);
    match serde_json::to_vec(&response) {
        Ok(body) if service
            .runtime_v3
            .validate_response(super::RuntimeV3GameplayRoute::DispatchAction, request, &body)
            .is_ok() => (503, body),
        Ok(_) | Err(_) => (503, b"{\"error\":\"recovery_translation_failed\"}".to_vec()),
    }
}
