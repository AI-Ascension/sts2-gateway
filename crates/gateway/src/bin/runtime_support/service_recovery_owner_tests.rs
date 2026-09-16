// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{
    GatewayRecoveryStore, RecoveryBootState, RecoveryContinuationOwner, RecoveryLeaseRequest,
    RecoveryReleaseSet, sha256_hex,
};
use uuid::Uuid;

use super::super::super::auth::AuthPolicy;
use super::super::super::continuation_owner::{CONTRACT, ContinuationOwnerKind, SCHEMA_DIGEST};
use super::super::super::http::HttpRequest;
use super::super::RuntimeService;
use super::super::test_support::test_service;

const ARTIFACT_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contract-artifact/continuation-owner-v1"
);
const SCHEMA_SHA256: &str = "5e787126c98cf950b94dcb4e02c5520ebbc5e0571a5e827cf0bd286815cd49dc";
const MANIFEST_SHA256: &str = "b23c9d4c4467806871e09b4bdb7919b721be79ee9e5ca40fce38ff63d67f2c0b";
const READ_PATH: &str = "/v1/recovery/continuation/owner/read";
const CLAIM_PATH: &str = "/v1/recovery/continuation/owner/claim";
const LOOKUP_PATH: &str = "/v1/recovery/continuation/owner/lookup";
const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000011";

struct ReadyService {
    service: RuntimeService,
    path: PathBuf,
    owner: RecoveryContinuationOwner,
}

fn ready_service() -> Result<ReadyService, String> {
    let mut service = test_service()?;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("recovery-token", None, None, "read,control")?;
    service.config.instance_id = Uuid::new_v4().to_string();
    service.config.caller_id = Uuid::new_v4().to_string();
    service.config.session_id = format!("session-{}", Uuid::new_v4());
    service.config.recovery_deployment_id = Some(DEPLOYMENT.to_owned());

    let path = std::env::temp_dir().join(format!(
        "sts2-gateway-owner-route-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let now = service.recovery_now_millis();
    let mut boot = store
        .start_boot(
            DEPLOYMENT,
            &service.config.instance_id,
            RecoveryReleaseSet::unconfigured(),
            now,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, now + 1)
        .map_err(|error| error.to_string())?;
    boot.state = RecoveryBootState::Ready;
    let lease = store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id: DEPLOYMENT.to_owned(),
            instance_id: service.config.instance_id.clone(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id: service.config.caller_id.clone(),
            session_id: service.config.session_id.clone(),
            now_millis: now + 2,
            ttl_seconds: service.config.recovery_ttl_seconds,
            renewal_interval_seconds: service.config.recovery_renewal_interval_seconds,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant = super::super::host_lease_helpers::grant_value(
        &boot,
        &fence,
        &lease,
        &service.config.caller_id,
        &service.config.session_id,
    );
    let grant_digest = super::super::super::host_lease_control::grant_digest(&grant)
        .map_err(|error| format!("host grant could not be digested: {error:?}"))?;
    store
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            &fence.host_fence_id,
            fence.fence_generation,
            now + 3,
        )
        .map_err(|error| error.to_string())?;
    store
        .complete_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            1,
            &Uuid::new_v4().to_string(),
            now + 4,
        )
        .map_err(|error| error.to_string())?;
    let owner = store
        .current_continuation_owner(&service.config.session_id, now + 5)
        .map_err(|error| error.to_string())?
        .owner
        .ok_or("expected owner")?;
    service.recovery = Some(store);
    service.recovery_boot = Some(boot);
    service.recovery_fence = Some(fence);
    service.recovery_lease = Some(lease.clone());
    service.recovery_host_grant = Some(super::super::HostLeaseGrant {
        installation_id,
        grant_digest,
        grant,
    });
    service.lease_active = true;
    service.recovery_lease_deadline = Some(Instant::now() + Duration::from_secs(30));
    service.recovery_lease_deadline_lease_id = Some(lease.lease_id.clone());
    if !service
        .active_host_grant_matches(&lease)
        .map_err(|error| error.to_string())?
    {
        return Err(String::from("ready fixture host grant is not canonical"));
    }
    Ok(ReadyService {
        service,
        path,
        owner,
    })
}

fn request(
    kind: ContinuationOwnerKind,
    payload: Value,
    principal_id: &str,
) -> Result<HttpRequest, String> {
    let (path, capability) = match kind {
        ContinuationOwnerKind::Read => (READ_PATH, kind.capability()),
        ContinuationOwnerKind::Claim => (CLAIM_PATH, kind.capability()),
        ContinuationOwnerKind::Lookup => (LOOKUP_PATH, kind.capability()),
    };
    let value = json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": Uuid::new_v4().to_string(),
        "actor": {"principal_id": principal_id, "role": "harness"},
        "auth": {"principal_id": principal_id, "capability": capability, "proof": null},
        "kind": match kind {
            ContinuationOwnerKind::Read => "current_owner_request",
            ContinuationOwnerKind::Claim => "owner_claim_request",
            ContinuationOwnerKind::Lookup => "owner_claim_lookup_request",
        },
        "payload": payload,
    });
    let body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    let mut headers = std::collections::BTreeMap::new();
    headers.insert(
        "authorization".to_owned(),
        "Bearer recovery-token".to_owned(),
    );
    headers.insert("content-type".to_owned(), "application/json".to_owned());
    headers.insert(
        "x-sts2-recovery-capability".to_owned(),
        capability.to_owned(),
    );
    Ok(HttpRequest {
        method: "POST".to_owned(),
        path: path.to_owned(),
        headers,
        body,
    })
}

fn response_value(body: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(body).map_err(|error| error.to_string())
}

fn assert_contract_valid(validator: &jsonschema::Validator, value: &Value, label: &str) {
    let errors = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{label}: {errors:?}");
}

fn contract_validator() -> Result<jsonschema::Validator, String> {
    let bytes = std::fs::read(PathBuf::from(ARTIFACT_ROOT).join("frame.schema.json"))
        .map_err(|error| error.to_string())?;
    let schema: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    jsonschema::validator_for(&schema).map_err(|error| error.to_string())
}

fn remove_database(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
}

#[test]
fn versioned_owner_contract_is_pinned_and_matches_closed_frames() -> Result<(), String> {
    let root = PathBuf::from(ARTIFACT_ROOT);
    let schema_bytes =
        std::fs::read(root.join("frame.schema.json")).map_err(|error| error.to_string())?;
    let manifest_bytes =
        std::fs::read(root.join("manifest.json")).map_err(|error| error.to_string())?;
    assert_eq!(sha256_hex(&schema_bytes), SCHEMA_SHA256);
    assert_eq!(sha256_hex(&manifest_bytes), MANIFEST_SHA256);
    let schema: Value = serde_json::from_slice(&schema_bytes).map_err(|error| error.to_string())?;
    let manifest: Value =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    assert_eq!(manifest["schema_digest"], SCHEMA_DIGEST);
    let validator = jsonschema::validator_for(&schema).map_err(|error| error.to_string())?;

    let ready = ready_service()?;
    for (kind, payload) in [
        (ContinuationOwnerKind::Read, json!({})),
        (
            ContinuationOwnerKind::Claim,
            json!({
                "operation_id": Uuid::new_v4().to_string(),
                "expected_owner": serde_json::to_value(&ready.owner).map_err(|e| e.to_string())?,
            }),
        ),
        (
            ContinuationOwnerKind::Lookup,
            json!({"operation_id": Uuid::new_v4().to_string()}),
        ),
    ] {
        let request = request(kind, payload, &ready.service.config.caller_id)?;
        let frame: Value =
            serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;
        assert_contract_valid(&validator, &frame, &format!("{kind:?}"));
    }
    for fixture in [
        "read-request.json",
        "claim-request.json",
        "historical-lookup-response.json",
    ] {
        let bytes = std::fs::read(root.join("fixtures").join(fixture))
            .map_err(|error| error.to_string())?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        assert_contract_valid(&validator, &value, fixture);
    }
    Ok(())
}

#[test]
fn production_dispatch_returns_non_secret_owner_and_idempotent_claim() -> Result<(), String> {
    let mut ready = ready_service()?;
    let validator = contract_validator()?;
    let read = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&read);
    assert_eq!(status, 200);
    let current = response_value(&body)?;
    assert_contract_valid(&validator, &current, "read response");
    assert_eq!(current["kind"], "current_owner_response");
    assert_eq!(current["payload"]["state"], "available");
    assert_eq!(
        current["payload"]["owner"]["lease_id"],
        ready.owner.lease_id
    );
    assert!(current["payload"]["owner"].get("fence_token").is_none());
    assert!(!String::from_utf8_lossy(&body).contains("lease_token"));

    let operation_id = Uuid::new_v4().to_string();
    let claim_payload = json!({
        "operation_id": operation_id,
        "expected_owner": serde_json::to_value(&ready.owner).map_err(|e| e.to_string())?,
    });
    let first = request(
        ContinuationOwnerKind::Claim,
        claim_payload.clone(),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&first);
    assert_eq!(status, 200);
    let first_value = response_value(&body)?;
    assert_contract_valid(&validator, &first_value, "claim response");
    assert_eq!(first_value["payload"]["result"], "CLAIMED");
    assert_eq!(
        first_value["payload"]["claim"]["operation_id"],
        operation_id
    );
    assert_eq!(
        first_value["payload"]["claim"]["owner"]["lease_id"],
        ready.owner.lease_id
    );

    let duplicate = request(
        ContinuationOwnerKind::Claim,
        claim_payload,
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&duplicate);
    assert_eq!(status, 200);
    let duplicate_value = response_value(&body)?;
    assert_contract_valid(&validator, &duplicate_value, "duplicate response");
    assert_eq!(duplicate_value["payload"]["result"], "DUPLICATE");
    assert_eq!(
        duplicate_value["payload"]["claim"]["request_digest"],
        first_value["payload"]["claim"]["request_digest"]
    );

    let sibling_payload = json!({
        "operation_id": Uuid::new_v4().to_string(),
        "expected_owner": serde_json::to_value(&ready.owner).map_err(|e| e.to_string())?,
    });
    let sibling = request(
        ContinuationOwnerKind::Claim,
        sibling_payload,
        &ready.service.config.caller_id,
    )?;
    assert_eq!(ready.service.handle_request(&sibling).0, 409);

    let mut stale_owner = ready.owner.clone();
    stale_owner.lease_epoch += 1;
    let stale = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": Uuid::new_v4().to_string(),
            "expected_owner": serde_json::to_value(stale_owner).map_err(|e| e.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    assert_eq!(ready.service.handle_request(&stale).0, 409);

    ready.service.recovery_lease = None;
    ready.service.lease_active = false;
    ready.service.recovery_lease_deadline = None;
    let lookup = request(
        ContinuationOwnerKind::Lookup,
        json!({"operation_id": operation_id}),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&lookup);
    assert_eq!(status, 200);
    let historical = response_value(&body)?;
    assert_contract_valid(&validator, &historical, "lookup response");
    assert_eq!(historical["payload"]["claim_state"], "historical");
    assert_eq!(historical["payload"]["current_owner"]["state"], "unknown");
    assert_eq!(
        historical["payload"]["claim"]["request_digest"],
        first_value["payload"]["claim"]["request_digest"]
    );
    let no_replay = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&ready.owner).map_err(|e| e.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    assert_eq!(ready.service.handle_request(&no_replay).0, 503);
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

include!("service_recovery_owner_readiness_tests.rs");

#[test]
fn owner_route_requires_recovery_scope_capability_and_closed_frame() -> Result<(), String> {
    let mut ready = ready_service()?;
    let mut missing_capability = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &ready.service.config.caller_id,
    )?;
    missing_capability
        .headers
        .remove("x-sts2-recovery-capability");
    assert_eq!(ready.service.handle_request(&missing_capability).0, 403);

    let wrong_principal = request(
        ContinuationOwnerKind::Read,
        json!({}),
        &Uuid::new_v4().to_string(),
    )?;
    assert_eq!(ready.service.handle_request(&wrong_principal).0, 403);

    let invalid = request(
        ContinuationOwnerKind::Read,
        json!({ "unexpected": true }),
        &ready.service.config.caller_id,
    )?;
    let (status, body) = ready.service.handle_request(&invalid);
    assert_eq!(status, 400);
    assert_eq!(
        response_value(&body)?["error_code"],
        "continuation_owner_frame_invalid"
    );
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}
