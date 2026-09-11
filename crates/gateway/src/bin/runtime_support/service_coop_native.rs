// SPDX-License-Identifier: MIT

use super::super::coop_native::CoopNativeRoute;
use super::super::coop_native_forwarder::{CoopNativeForwardError, CoopNativeForwarder};
use super::{
    CoopNativePeerBinding, CoopNativePendingOperation, HttpRequest, MAX_RESPONSE_BYTES,
    RuntimeService, json_error,
};
use serde_json::Value;

impl RuntimeService {
    pub(super) fn coop_native_request(
        &mut self,
        request: &HttpRequest,
        route: CoopNativeRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route != CoopNativeRoute::Observation && !request.content_type_is_json() {
            return (400, json_error("coop_native_content_type_required"));
        }
        let envelope =
            match self
                .coop_native
                .validate_request(route, &request.body, &request.headers)
            {
                Ok(envelope) => envelope,
                Err(error) => {
                    return (
                        request_error_status(error),
                        json_error(request_error_code(error)),
                    );
                }
            };
        if let Err(error) = self.validate_coop_native_binding(request, route, envelope.as_ref()) {
            return error;
        }
        if let Err(error) = self.admit_coop_native_operation(route, envelope.as_ref()) {
            return error;
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit(
            if route == CoopNativeRoute::Observation {
                "GET"
            } else {
                "POST"
            },
            route.downstream_path(),
            &request.body,
            correlation,
            MAX_RESPONSE_BYTES,
        ) {
            Ok(response) => response,
            Err(status) => return (status, json_error("coop_native_downstream_unavailable")),
        };
        if self
            .coop_native
            .validate_response(
                route,
                envelope.as_ref(),
                &request.headers,
                response.status,
                &response.body,
            )
            .is_ok()
        {
            if let Err(error) = self.validate_coop_native_return_local_peer(&response.body) {
                return error;
            }
            if let Err(error) =
                self.record_coop_native_return(route, envelope.as_ref(), &response.body)
            {
                return error;
            }
            return (response.status, response.body);
        }
        // The managed producer reports stale/admission failures as a bounded one-field HTTP
        // error rather than a protocol envelope. Preserve those explicit host decisions while
        // mapping every other malformed downstream response to 502.
        if CoopNativeForwarder::is_bounded_error(response.status, &response.body) {
            if matches!(response.status, 400 | 409) {
                self.clear_rejected_coop_native_pending(route, envelope.as_ref());
            }
            return (response.status, response.body);
        }
        (502, json_error("coop_native_response_invalid"))
    }
}

impl RuntimeService {
    fn validate_coop_native_binding(
        &self,
        request: &HttpRequest,
        route: CoopNativeRoute,
        envelope: Option<&Value>,
    ) -> Result<(), (u16, Vec<u8>)> {
        let Some(binding) = self.coop_native_peer_binding.as_ref() else {
            return Err((503, json_error("coop_native_peer_binding_unconfigured")));
        };
        if !binding.matches(request) {
            return Err((409, json_error("coop_native_route_binding_stale")));
        }
        if !constant_time_equal(
            request.headers.get("x-sts2-peer-token").map(String::as_str),
            &binding.peer_token,
        ) {
            return Err((401, json_error("coop_native_peer_unauthorized")));
        }
        if matches!(
            route,
            CoopNativeRoute::LegalCatalog
                | CoopNativeRoute::LocalAction
                | CoopNativeRoute::SharedVote
                | CoopNativeRoute::Rejoin
        ) && envelope.is_some_and(|value| value["actor_peer"].as_str() != Some(&binding.peer_id))
        {
            return Err((409, json_error("coop_native_peer_substitution_rejected")));
        }
        Ok(())
    }

    fn admit_coop_native_operation(
        &mut self,
        route: CoopNativeRoute,
        envelope: Option<&Value>,
    ) -> Result<(), (u16, Vec<u8>)> {
        let Some(envelope) = envelope else {
            return Ok(());
        };
        let Some(operation_id) = envelope["operation_id"].as_str() else {
            return Ok(());
        };
        match route {
            CoopNativeRoute::LocalAction
            | CoopNativeRoute::SharedVote
            | CoopNativeRoute::Rejoin => {
                if self.coop_native_pending.is_some() {
                    return Err((409, json_error("coop_native_operation_pending")));
                }
                let Some(binding) = self.coop_native_peer_binding.clone() else {
                    return Err((503, json_error("coop_native_peer_binding_unconfigured")));
                };
                self.coop_native_pending = Some(CoopNativePendingOperation {
                    route,
                    operation_id: operation_id.to_owned(),
                    binding,
                    authority_id: None,
                    authority_epoch: None,
                });
                Ok(())
            }
            CoopNativeRoute::Recover => {
                let Some(pending) = self.coop_native_pending.as_ref() else {
                    return Err((409, json_error("coop_native_operation_not_pending")));
                };
                if pending.operation_id != operation_id
                    || !self
                        .coop_native_peer_binding
                        .as_ref()
                        .is_some_and(|binding| same_binding(binding, &pending.binding))
                {
                    return Err((409, json_error("coop_native_recovery_binding_rejected")));
                }
                Ok(())
            }
            CoopNativeRoute::Observation | CoopNativeRoute::LegalCatalog => Ok(()),
        }
    }

    fn record_coop_native_return(
        &mut self,
        route: CoopNativeRoute,
        request: Option<&Value>,
        response: &[u8],
    ) -> Result<(), (u16, Vec<u8>)> {
        let Some(request) = request else {
            return Ok(());
        };
        let Some(operation_id) = request["operation_id"].as_str() else {
            return Ok(());
        };
        if !matches!(
            route,
            CoopNativeRoute::LocalAction
                | CoopNativeRoute::SharedVote
                | CoopNativeRoute::Rejoin
                | CoopNativeRoute::Recover
        ) {
            return Ok(());
        }
        let Some(pending) = self.coop_native_pending.as_ref() else {
            return Err((502, json_error("coop_native_return_binding_missing")));
        };
        if pending.operation_id != operation_id
            || (route != CoopNativeRoute::Recover && pending.route != route)
        {
            return Err((502, json_error("coop_native_return_binding_rejected")));
        }
        let value = super::super::strict_json::parse(response)
            .map_err(|_| (502, json_error("coop_native_response_invalid")))?;
        let authority_id = value["observation"]["authority_id"].as_str();
        let authority_epoch = value["observation"]["host_authority_epoch"].as_str();
        if let (Some(expected_id), Some(expected_epoch)) = (
            pending.authority_id.as_deref(),
            pending.authority_epoch.as_deref(),
        ) && (authority_id != Some(expected_id) || authority_epoch != Some(expected_epoch))
        {
            return Err((409, json_error("coop_native_authority_changed")));
        }
        let terminal = matches!(value["status"].as_str(), Some("settled" | "rejected"));
        if terminal {
            self.coop_native_pending = None;
        } else if let Some(pending) = self.coop_native_pending.as_mut() {
            pending.authority_id = authority_id.map(ToOwned::to_owned);
            pending.authority_epoch = authority_epoch.map(ToOwned::to_owned);
        }
        Ok(())
    }

    fn clear_rejected_coop_native_pending(
        &mut self,
        route: CoopNativeRoute,
        request: Option<&Value>,
    ) {
        let Some(operation_id) = request.and_then(|value| value["operation_id"].as_str()) else {
            return;
        };
        if self
            .coop_native_pending
            .as_ref()
            .is_some_and(|pending| pending.route == route && pending.operation_id == operation_id)
        {
            self.coop_native_pending = None;
        }
    }
}

impl CoopNativePeerBinding {
    fn matches(&self, request: &HttpRequest) -> bool {
        request
            .headers
            .get("x-sts2-instance-id")
            .map(String::as_str)
            == Some(&self.instance_id)
            && request.headers.get("x-sts2-session-id").map(String::as_str)
                == Some(&self.session_id)
            && request.headers.get("x-sts2-lease-id").map(String::as_str) == Some(&self.lease_id)
            && request
                .headers
                .get("x-sts2-lease-epoch")
                .and_then(|value| value.parse::<u64>().ok())
                == Some(self.lease_epoch)
    }
}

fn same_binding(left: &CoopNativePeerBinding, right: &CoopNativePeerBinding) -> bool {
    constant_time_equal(Some(&left.peer_token), &right.peer_token)
        && left.peer_id == right.peer_id
        && left.instance_id == right.instance_id
        && left.session_id == right.session_id
        && left.lease_id == right.lease_id
        && left.lease_epoch == right.lease_epoch
}

fn constant_time_equal(provided: Option<&str>, expected: &str) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .bytes()
        .zip(expected.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn request_error_status(error: CoopNativeForwardError) -> u16 {
    match error {
        CoopNativeForwardError::RequestBodyOversized => 413,
        CoopNativeForwardError::RequestBodyRequired
        | CoopNativeForwardError::RequestBodyForbidden
        | CoopNativeForwardError::RequestBodyMalformed => 400,
        CoopNativeForwardError::ResponseOversized | CoopNativeForwardError::ResponseMalformed => {
            502
        }
    }
}

fn request_error_code(error: CoopNativeForwardError) -> &'static str {
    match error {
        CoopNativeForwardError::RequestBodyRequired => "coop_native_body_required",
        CoopNativeForwardError::RequestBodyForbidden => "coop_native_body_forbidden",
        CoopNativeForwardError::RequestBodyOversized => "coop_native_body_oversized",
        CoopNativeForwardError::RequestBodyMalformed => "coop_native_request_invalid",
        CoopNativeForwardError::ResponseOversized => "coop_native_response_oversized",
        CoopNativeForwardError::ResponseMalformed => "coop_native_response_invalid",
    }
}
