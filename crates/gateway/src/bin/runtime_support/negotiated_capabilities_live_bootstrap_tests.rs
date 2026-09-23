// SPDX-License-Identifier: MIT

use super::super::super::game_information_lookup_binding::{
    BoundLookupBinding, LookupBindingScope,
};
use super::super::test_support::test_service;
use super::tests::install_capabilities;
use super::tests::request;
use super::{CAPABILITIES_VERSION_HEADER, SCHEMA_VERSION, SCHEMA_VERSION_V2};
use serde_json::Value;

const V2_SCHEMA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/negotiated-capabilities-v2/schema.json"
));
const V2_SCHEMA_DIGEST: &str = "c6453f1a760675c7492261eb7b50be76cf87d8c7d4762a070d26693b15225b7f";

fn install_live_binding(service: &mut super::super::RuntimeService) {
    let authority = service.game_information_authority();
    service.game_information_lookup_binding = Some(BoundLookupBinding {
        authority,
        binding_id: String::from("binding-1"),
        content_manifest_id: String::from("content-1"),
        authority_epoch: 7,
        scope: LookupBindingScope {
            project_id: String::from("proj-1"),
            run_id: String::from("run-1"),
            episode_id: String::from("episode-7"),
            agent_id: String::from("agent-3"),
            authority_epoch: 7,
        },
    });
}

fn v2_request() -> super::super::super::http::HttpRequest {
    let mut request = request();
    request.headers.insert(
        String::from(CAPABILITIES_VERSION_HEADER),
        String::from(SCHEMA_VERSION_V2),
    );
    request
}

#[test]
fn live_bootstrap_offer_requires_enabled_profile_and_current_binding() -> Result<(), String> {
    let mut service = test_service()?;
    install_capabilities(&mut service);
    let (status, body) = service.handle_request(&request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(!value["offers"].as_array().is_some_and(|offers| {
        offers
            .iter()
            .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
    }));

    service.config.game_information_live_bootstrap_enabled = true;
    service.game_information_live_bootstrap_supported = Some(service.game_information_authority());
    install_live_binding(&mut service);
    let (status, body) = service.handle_request(&v2_request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["schema_version"], SCHEMA_VERSION_V2);
    let offer = value["offers"]
        .as_array()
        .and_then(|offers| {
            offers
                .iter()
                .find(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
        })
        .ok_or("live bootstrap offer missing")?;
    assert_eq!(
        offer["revision"],
        "game-information-live-observation-bootstrap-v1"
    );
    Ok(())
}

#[test]
fn live_bootstrap_offer_with_wrong_owner_is_withheld() -> Result<(), String> {
    let mut service = test_service()?;
    install_capabilities(&mut service);
    service.config.game_information_live_bootstrap_enabled = true;
    install_live_binding(&mut service);
    service
        .game_information_lookup_binding
        .as_mut()
        .ok_or("binding missing")?
        .authority
        .lease_id = String::from("foreign-lease");
    let (status, body) = service.handle_request(&v2_request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(!value["offers"].as_array().is_some_and(|offers| {
        offers
            .iter()
            .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
    }));
    Ok(())
}

#[test]
fn failed_handler_stays_withdrawn_until_authority_changes() -> Result<(), String> {
    let mut service = test_service()?;
    install_capabilities(&mut service);
    service.config.game_information_live_bootstrap_enabled = true;
    install_live_binding(&mut service);
    service.game_information_live_bootstrap_supported = Some(service.game_information_authority());
    service.game_information_live_bootstrap_transport_failed = true;
    let (status, body) = service.handle_request(&v2_request());
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(!value["offers"].as_array().is_some_and(|offers| {
        offers
            .iter()
            .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
    }));

    service.config.lease_id = String::from("lease-rotated");
    install_capabilities(&mut service);
    let mut rotated = request();
    rotated.headers.insert(
        String::from("x-sts2-lease-id"),
        String::from("lease-rotated"),
    );
    rotated.headers.insert(
        String::from(CAPABILITIES_VERSION_HEADER),
        String::from(SCHEMA_VERSION_V2),
    );
    let (status, body) = service.handle_request(&rotated);
    assert_eq!(status, 200);
    assert_eq!(
        service
            .game_information_live_bootstrap_supported
            .as_ref()
            .map(|authority| authority.lease_id.as_str()),
        Some("lease-rotated")
    );
    assert!(!service.game_information_live_bootstrap_transport_failed);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert!(!value["offers"].as_array().is_some_and(|offers| {
        offers
            .iter()
            .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
    }));
    Ok(())
}

#[test]
fn default_and_explicit_v1_never_advertise_live_bootstrap() -> Result<(), String> {
    let mut service = test_service()?;
    install_capabilities(&mut service);
    service.config.game_information_live_bootstrap_enabled = true;
    service.game_information_live_bootstrap_supported = Some(service.game_information_authority());
    install_live_binding(&mut service);

    for version in [None, Some(SCHEMA_VERSION)] {
        let mut request = request();
        if let Some(version) = version {
            request.headers.insert(
                String::from(CAPABILITIES_VERSION_HEADER),
                String::from(version),
            );
        }
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 200);
        let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert!(!value["offers"].as_array().is_some_and(|offers| {
            offers
                .iter()
                .any(|offer| offer["operation"] == "game_information.live_observation_bootstrap")
        }));
    }
    Ok(())
}

#[test]
fn unsupported_capabilities_version_is_rejected_without_fallback() -> Result<(), String> {
    let mut service = test_service()?;
    install_capabilities(&mut service);
    let mut request = request();
    request.headers.insert(
        String::from(CAPABILITIES_VERSION_HEADER),
        String::from("sts2-gateway-negotiated-capabilities-v9"),
    );
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 406);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        "negotiated_capabilities_version_unsupported"
    );
    Ok(())
}

#[test]
fn negotiated_v2_schema_digest_is_pinned() {
    assert_eq!(sts2_gateway::sha256_hex(V2_SCHEMA), V2_SCHEMA_DIGEST);
}
