// SPDX-License-Identifier: MIT

//! Gateway-owned, authenticated capability evidence for the MCP composition
//! profile. This deliberately reports only producer support the gateway has
//! already validated; a compiled forwarder is never an offer.

use serde_json::{Value, json};

use super::*;

pub(crate) const SCHEMA_VERSION: &str = "sts2-gateway-negotiated-capabilities-v1";
pub(crate) const SCHEMA_VERSION_V2: &str = "sts2-gateway-negotiated-capabilities-v2";
pub(crate) const CAPABILITIES_VERSION_HEADER: &str = "x-sts2-capabilities-version";
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundRuntimeV3Baseline {
    authority: super::game_information_forwarder::GameInformationProducerAuthority,
}
const GAME_INFORMATION_PROFILE: &str = "game-information-query-v1";
const GAME_INFORMATION_SCHEMA_DIGEST: &str =
    "376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9";
const LOOKUP_BINDING_PROFILE: &str = "game-information-lookup-binding-v1";
const LOOKUP_BINDING_SCHEMA_DIGEST: &str =
    "f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58";
const RUNTIME_V3_BASELINE_PROFILE: &str = "runtime-v3-gameplay";
const RUNTIME_V3_BASELINE_SCHEMA_DIGEST: &str =
    "daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b";
const RUNTIME_V3_STATE_REQUEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v3-gameplay/golden/state-request.json"
));

pub(crate) fn path(instance_id: &str) -> String {
    format!("/v1/instances/{instance_id}/negotiated-capabilities")
}

impl RuntimeService {
    pub(super) fn negotiated_capabilities(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if !request.body.is_empty() {
            return (400, json_error("negotiated_capabilities_body_forbidden"));
        }
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let v2 = match request
            .headers
            .get(CAPABILITIES_VERSION_HEADER)
            .map(String::as_str)
        {
            None | Some(SCHEMA_VERSION) => false,
            Some(SCHEMA_VERSION_V2) => true,
            Some(_) => {
                return (
                    406,
                    json_error("negotiated_capabilities_version_unsupported"),
                );
            }
        };
        let authority = self.game_information_authority();
        if self.game_information_live_bootstrap_supported.as_ref() != Some(&authority) {
            self.game_information_live_bootstrap_supported = self
                .config
                .game_information_live_bootstrap_enabled
                .then_some(authority.clone());
            self.game_information_live_bootstrap_transport_failed = false;
        }
        // Capability discovery is part of the authenticated startup path. Refresh a
        // missing or authority-stale cache through the same validated producer
        // exchange used by the public game-information capability endpoint.
        let current = self
            .game_information_capabilities
            .as_ref()
            .is_some_and(|bound| bound.authority == self.game_information_authority());
        if !current {
            let cancellation = super::RequestCancellation::new();
            let (status, body) = self.game_information_capabilities(request, &cancellation);
            if status != 200 {
                return (status, body);
            }
        }
        let Some(bound) = self.game_information_capabilities.as_ref() else {
            return (503, json_error("negotiated_capabilities_unavailable"));
        };
        if bound.authority != self.game_information_authority() {
            return (503, json_error("negotiated_capabilities_unavailable"));
        }
        let capabilities = bound.capabilities.clone();
        let capability_authority = bound.authority.clone();
        let provided = request.headers.get("authorization").map(String::as_str);
        let scope_bits = match self.config.auth_policy.authorized_scope_bits(provided) {
            Ok(bits) => bits,
            Err(_) => return (401, json_error("unauthorized")),
        };
        let scopes = scope_names(scope_bits);
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let capabilities = &capabilities;
        // The producer capability endpoint itself proves this known operation.
        // Query kinds remain a closed mapping; this route never reflects arbitrary
        // producer operation strings into the negotiated catalog.
        let mut offers = vec![known_game_information_capabilities_offer(
            capabilities,
            &scopes,
        )];
        offers.extend(
            capabilities
                .query_kinds
                .iter()
                .filter_map(|kind| known_game_information_offer(kind, capabilities, &scopes)),
        );
        self.refresh_runtime_v3_baseline(request);
        let runtime_v3_baseline = self
            .runtime_v3_baseline
            .as_ref()
            .filter(|bound| bound.authority == self.game_information_authority());
        if runtime_v3_baseline.is_some() {
            offers.extend(known_runtime_v3_baseline_offers(&scopes, scope_bits));
        }
        let runtime_v3_baseline_witness = runtime_v3_baseline.map(|_| {
            json!({
                "profile": RUNTIME_V3_BASELINE_PROFILE,
                "schema_digest": RUNTIME_V3_BASELINE_SCHEMA_DIGEST,
                "configured_state_probe": true,
                "recovery_kinds": ["reobserve", "reconcile"],
            })
        });
        let lookup_binding = self
            .game_information_lookup_binding
            .as_ref()
            .filter(|bound| {
                bound.authority == self.game_information_authority()
                    && bound.content_manifest_id
                        == self.game_information_authority().content_manifest_id
            });
        if lookup_binding.is_some() {
            offers.extend(known_lookup_binding_offers(&scopes));
            if self
                .game_information_live_bootstrap_supported
                .as_ref()
                .is_some_and(|support| {
                    support == &self.game_information_authority()
                        && !self.game_information_live_bootstrap_transport_failed
                })
                && v2
            {
                offers.push(known_live_observation_bootstrap_offer(
                    &scopes,
                    super::super::game_information_live_observation_bootstrap::MAX_REQUEST_BYTES,
                    super::super::game_information_live_observation_bootstrap::MAX_BOOTSTRAP_RESPONSE_BYTES,
                ));
            }
        }
        let lookup_binding_witness = lookup_binding.map(|bound| {
            json!({
                "profile": LOOKUP_BINDING_PROFILE,
                "schema_digest": LOOKUP_BINDING_SCHEMA_DIGEST,
                "binding_id": bound.binding_id,
                "content_manifest_id": bound.content_manifest_id,
                "authority_epoch": bound.authority_epoch,
            })
        });
        let value = json!({
            "schema_version": if v2 { SCHEMA_VERSION_V2 } else { SCHEMA_VERSION },
            "gateway_revision": if v2 { SCHEMA_VERSION_V2 } else { SCHEMA_VERSION },
            "correlation_id": correlation_id,
            "instance_id": capability_authority.instance_id,
            "caller_id": capability_authority.caller_id,
            "session_id": capability_authority.session_id,
            "mcp_session_id": self.config.mcp_session_id,
            "lease_id": capability_authority.lease_id,
            "lease_epoch": capability_authority.lease_epoch,
            "caller_scopes": scopes,
            "producer": {
                "profile": GAME_INFORMATION_PROFILE,
                "schema_digest": GAME_INFORMATION_SCHEMA_DIGEST,
                "content_manifest_id": capability_authority.content_manifest_id,
                "run_id": capability_authority.run_id,
            },
            "lookup_binding_witness": lookup_binding_witness,
            "runtime_v3_baseline_witness": runtime_v3_baseline_witness,
            "offers": offers,
        });
        let bytes = json_bytes(&value);
        if bytes.len() > MAX_RESPONSE_BYTES {
            return (503, json_error("negotiated_capabilities_oversized"));
        }
        (200, bytes)
    }

    fn refresh_runtime_v3_baseline(&mut self, snapshot_request: &HttpRequest) {
        let current = self
            .runtime_v3_baseline
            .as_ref()
            .is_some_and(|bound| bound.authority == self.game_information_authority());
        if current {
            return;
        }
        self.runtime_v3_baseline = None;
        let Some(body) =
            runtime_v3_state_request(snapshot_request, &self.game_information_authority())
        else {
            return;
        };
        let mut headers = snapshot_request.headers.clone();
        headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        let request = HttpRequest {
            method: String::from("GET"),
            path: format!("/v3/instances/{}/state", self.config.instance_id),
            headers,
            body,
        };
        if self
            .runtime_v3_request(&request, super::RuntimeV3GameplayRoute::State)
            .0
            == 200
        {
            self.runtime_v3_baseline = Some(BoundRuntimeV3Baseline {
                authority: self.game_information_authority(),
            });
        }
    }
}

fn scope_names(bits: u8) -> Vec<&'static str> {
    let mut names = vec!["read"];
    if bits & 0b010 != 0 {
        names.push("mutate");
    }
    if bits & 0b100 != 0 {
        names.push("control");
    }
    names
}

fn runtime_v3_state_request(
    request: &HttpRequest,
    authority: &super::game_information_forwarder::GameInformationProducerAuthority,
) -> Option<Vec<u8>> {
    let correlation_id = request.headers.get("x-sts2-correlation-id")?;
    let mut value: Value = serde_json::from_slice(RUNTIME_V3_STATE_REQUEST).ok()?;
    value["correlation_id"] = Value::String(correlation_id.clone());
    value["instance_id"] = Value::String(authority.instance_id.clone());
    value["session_id"] = Value::String(authority.session_id.clone());
    value["lease_id"] = Value::String(authority.lease_id.clone());
    value["lease_epoch"] = Value::from(authority.lease_epoch);
    serde_json::to_vec(&value).ok()
}

#[path = "negotiated_capabilities_offers.rs"]
mod offers;
use offers::{
    known_game_information_capabilities_offer, known_game_information_offer,
    known_live_observation_bootstrap_offer, known_lookup_binding_offers,
    known_runtime_v3_baseline_offers,
};

#[cfg(test)]
#[path = "negotiated_capabilities_baseline_tests.rs"]
mod baseline_tests;
#[cfg(test)]
#[path = "negotiated_capabilities_live_bootstrap_tests.rs"]
mod live_bootstrap_tests;
#[cfg(test)]
#[path = "negotiated_capabilities_manifest_tests.rs"]
mod manifest_tests;
#[cfg(test)]
#[path = "negotiated_capabilities_tests.rs"]
mod tests;
