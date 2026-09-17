// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::game_information_forwarder::{GameInformationProducerAuthority, MAX_RESPONSE_BYTES};
use super::strict_json;

pub(crate) const SCHEMA_DIGEST: &str =
    "f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58";
const PROTOCOL_VERSION: &str = "game-information-lookup-binding-v1";
const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-lookup-binding-v1/schema.json"
));
pub(crate) const RESPONSE_LIMIT_BYTES: usize = MAX_RESPONSE_BYTES;

/// Gateway-local evidence produced only after a pinned discovery response has
/// passed the complete lookup-binding validator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundLookupBinding {
    pub(crate) authority: GameInformationProducerAuthority,
    pub(crate) binding_id: String,
    pub(crate) content_manifest_id: String,
    pub(crate) authority_epoch: u64,
    pub(crate) scope: LookupBindingScope,
}

/// The closed harness-owned request identity retained solely to prove that a
/// later observation is for the exact discovery binding before Gateway I/O.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LookupBindingScope {
    pub(crate) project_id: String,
    pub(crate) run_id: String,
    pub(crate) episode_id: String,
    pub(crate) agent_id: String,
    pub(crate) authority_epoch: u64,
}

pub(crate) fn discovery_witness(
    response_body: &[u8],
    request_body: &[u8],
) -> Option<(String, String, u64, LookupBindingScope)> {
    let request = strict_json::parse(request_body).ok()?;
    if request.get("operation").and_then(Value::as_str) != Some("discovery") {
        return None;
    }
    let (binding_id, content_manifest_id, authority_epoch) = response_binding(response_body)?;
    Some((
        binding_id,
        content_manifest_id,
        authority_epoch,
        request_scope(request_body)?,
    ))
}

/// This is called only after the route has accepted the closed request shape.
pub(crate) fn request_scope(request_body: &[u8]) -> Option<LookupBindingScope> {
    let request = strict_json::parse(request_body).ok()?;
    Some(LookupBindingScope {
        project_id: request.get("project_id")?.as_str()?.to_owned(),
        run_id: request.get("run_id")?.as_str()?.to_owned(),
        episode_id: request.get("episode_id")?.as_str()?.to_owned(),
        agent_id: request.get("agent_id")?.as_str()?.to_owned(),
        authority_epoch: request.get("authority_epoch")?.as_u64()?,
    })
}

/// Extracts the closed binding identity after `response_is_valid` has checked
/// the response kind, request scope, canonical digest, and observation link.
pub(crate) fn response_binding(response_body: &[u8]) -> Option<(String, String, u64)> {
    let response = strict_json::parse(response_body).ok()?;
    let binding = response.get("binding")?.as_object()?;
    Some((
        binding.get("binding_id")?.as_str()?.to_owned(),
        binding.get("content_manifest_id")?.as_str()?.to_owned(),
        binding.get("authority_epoch")?.as_u64()?,
    ))
}

pub(crate) fn response_is_valid(
    body: &[u8],
    request_body: &[u8],
    correlation: &str,
    instance_id: &str,
    locale: &str,
    status: u16,
) -> bool {
    if body.len() > RESPONSE_LIMIT_BYTES {
        return false;
    }
    let Some(validator) = schema_validator() else {
        return false;
    };
    let Ok(request) = strict_json::parse(request_body) else {
        return false;
    };
    let Ok(response) = strict_json::parse(body) else {
        return false;
    };
    if !validator.is_valid(&response)
        || response.get("protocol_version").and_then(Value::as_str) != Some(PROTOCOL_VERSION)
        || response.get("schema_digest").and_then(Value::as_str) != Some(SCHEMA_DIGEST)
        || response.get("correlation_id").and_then(Value::as_str) != Some(correlation)
    {
        return false;
    }

    let Some(operation) = request.get("operation").and_then(Value::as_str) else {
        return false;
    };
    let Some(kind) = response.get("kind").and_then(Value::as_str) else {
        return false;
    };
    let typed_error = kind == "error_response";
    let matching_operation = typed_error
        || matches!(
            (operation, kind),
            ("discovery", "lookup_binding_discovery_response")
                | ("observe", "lookup_binding_observation_response")
        );
    if !matching_operation
        || (typed_error && !(400..=599).contains(&status))
        || (!typed_error && status != 200)
    {
        return false;
    }

    let binding = response.get("binding");
    if let Some(binding) = binding.filter(|binding| !binding.is_null()) {
        if !binding_matches_request(binding, &request, instance_id, locale) {
            return false;
        }
        let Some(binding_id) = binding.get("binding_id").and_then(Value::as_str) else {
            return false;
        };
        if canonical_binding_id(binding, &request).as_deref() != Some(binding_id) {
            return false;
        }
        if response
            .get("observation")
            .and_then(Value::as_object)
            .is_some_and(|observation| {
                observation.get("binding_id").and_then(Value::as_str) != Some(binding_id)
            })
        {
            return false;
        }
    } else if !typed_error {
        return false;
    }

    response
        .get("discovery")
        .and_then(Value::as_object)
        .is_none_or(|discovery| {
            discovery
                .get("required_capabilities")
                .and_then(Value::as_object)
                .is_some_and(|capabilities| {
                    capabilities.get("profile").and_then(Value::as_str) == Some(PROTOCOL_VERSION)
                        && capabilities.get("schema_digest").and_then(Value::as_str)
                            == Some(SCHEMA_DIGEST)
                })
        })
}

fn binding_matches_request(
    binding: &Value,
    request: &Value,
    instance_id: &str,
    locale: &str,
) -> bool {
    let Some(binding_scope) = binding.get("scope").and_then(Value::as_object) else {
        return false;
    };
    let Some(request_epoch) = request.get("authority_epoch") else {
        return false;
    };
    ["project_id", "run_id", "episode_id", "agent_id"]
        .into_iter()
        .all(|name| binding_scope.get(name) == request.get(name))
        && binding.get("authority_epoch") == Some(request_epoch)
        && binding.get("instance_id").and_then(Value::as_str) == Some(instance_id)
        && binding.get("locale").and_then(Value::as_str) == Some(locale)
}

fn canonical_binding_id(binding: &Value, request: &Value) -> Option<String> {
    let scope = binding.get("scope")?;
    let mut identity = BTreeMap::<&str, Value>::new();
    for (name, source) in [
        ("agent_id", scope.get("agent_id")?),
        ("authority_epoch", request.get("authority_epoch")?),
        ("content_manifest_id", binding.get("content_manifest_id")?),
        ("episode_id", scope.get("episode_id")?),
        ("game_profile", binding.get("game_profile")?),
        ("locale", binding.get("locale")?),
        ("project_id", scope.get("project_id")?),
        ("run_id", scope.get("run_id")?),
    ] {
        identity.insert(name, source.clone());
    }
    let bytes = serde_json::to_vec(&identity).ok()?;
    Some(sts2_gateway::sha256_hex(&bytes))
}

fn schema_validator() -> Option<&'static jsonschema::Validator> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            if sts2_gateway::sha256_hex(SCHEMA.as_bytes()) != SCHEMA_DIGEST {
                return None;
            }
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
}
