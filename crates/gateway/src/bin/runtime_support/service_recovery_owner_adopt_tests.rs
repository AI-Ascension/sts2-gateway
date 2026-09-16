// SPDX-License-Identifier: MIT

use std::net::TcpListener;

const ADOPT_ARTIFACT_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contract-artifact/continuation-owner-adopt-v1"
);
const ADOPT_SCHEMA_DIGEST: &str =
    "7240e2f5054e5f6639ad12fa1ed66d40992f1234e49b693b2aacc53451dec380";
const ADOPT_PATH: &str = "/v1/recovery/continuation/owner/adopt";

fn adoption_request(
    operation_id: &str,
    owner: &RecoveryContinuationOwner,
    principal_id: &str,
) -> Result<HttpRequest, String> {
    let value = json!({
        "contract": super::super::super::continuation_owner_adopt::CONTRACT,
        "schema_digest": ADOPT_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": Uuid::new_v4().to_string(),
        "actor": {"principal_id": principal_id, "role": "harness"},
        "auth": {
            "principal_id": principal_id,
            "capability": "continuation_owner_adopt",
            "proof": null
        },
        "kind": "owner_adopt_request",
        "payload": {
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(owner).map_err(|error| error.to_string())?,
        }
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
        "continuation_owner_adopt".to_owned(),
    );
    Ok(HttpRequest {
        method: "POST".to_owned(),
        path: ADOPT_PATH.to_owned(),
        headers,
        body,
    })
}

fn adoption_contract_validator() -> Result<jsonschema::Validator, String> {
    let bytes = std::fs::read(PathBuf::from(ADOPT_ARTIFACT_ROOT).join("frame.schema.json"))
        .map_err(|error| error.to_string())?;
    let schema: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    jsonschema::validator_for(&schema).map_err(|error| error.to_string())
}

#[test]
fn adoption_contract_is_pinned_and_accepts_its_golden_frames() -> Result<(), String> {
    let root = PathBuf::from(ADOPT_ARTIFACT_ROOT);
    let schema_bytes =
        std::fs::read(root.join("frame.schema.json")).map_err(|error| error.to_string())?;
    let manifest_bytes =
        std::fs::read(root.join("manifest.json")).map_err(|error| error.to_string())?;
    assert_eq!(sha256_hex(&schema_bytes), ADOPT_SCHEMA_DIGEST);
    let schema: Value =
        serde_json::from_slice(&schema_bytes).map_err(|error| error.to_string())?;
    let manifest: Value =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    assert_eq!(manifest["schema_digest"], ADOPT_SCHEMA_DIGEST);
    assert_eq!(manifest["contract"], "sts2-continuation-owner-adopt-v1");
    let validator = jsonschema::validator_for(&schema).map_err(|error| error.to_string())?;
    for fixture in ["adopt-request.json", "adopt-response.json"] {
        let bytes = std::fs::read(root.join("fixtures").join(fixture))
            .map_err(|error| error.to_string())?;
        let frame: Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        assert_contract_valid(&validator, &frame, fixture);
    }
    Ok(())
}

#[test]
fn live_owner_adoption_replays_same_claim_and_durable_authority() -> Result<(), String> {
    let mut ready = ready_service()?;
    let downstream = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    downstream
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    ready.service.config.mod_address = downstream
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let validator = adoption_contract_validator()?;
    let operation_id = Uuid::new_v4().to_string();
    let claim_request = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&ready.owner).map_err(|error| error.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    let (claim_status, claim_body) = ready.service.handle_request(&claim_request);
    assert_eq!(claim_status, 200);
    let claim = response_value(&claim_body)?["payload"]["claim"].clone();
    let owner_before = ready.owner.clone();
    let request = adoption_request(
        &operation_id,
        &owner_before,
        &ready.service.config.caller_id,
    )?;
    let request_value: Value =
        serde_json::from_slice(&request.body).map_err(|error| error.to_string())?;

    let (status, body) = ready.service.handle_request(&request);
    assert_eq!(status, 200);
    assert!(
        matches!(
            downstream.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ),
        "owner adoption must not forward to the native runtime"
    );
    let response = response_value(&body)?;
    assert_contract_valid(&validator, &response, "adoption response");
    assert_eq!(response["correlation_id"], request_value["correlation_id"]);
    assert_eq!(response["payload"]["result"], "ADOPTED");
    assert_eq!(response["payload"]["claim"], claim);
    assert_eq!(
        response["payload"]["owner"],
        serde_json::to_value(&owner_before).map_err(|error| error.to_string())?
    );
    let authority = &response["payload"]["recovery_authority"];
    assert_eq!(
        authority["contract"],
        "watchdog-runtime-allocation-v1"
    );
    assert_eq!(authority["context"]["lease_id"], owner_before.lease_id);
    assert_eq!(authority["context"]["lease_epoch"], owner_before.lease_epoch);
    assert_eq!(
        authority["current_fence"]["created_at"],
        super::super::super::recovery_frame::timestamp_from_millis(
            ready
                .service
                .recovery_fence
                .as_ref()
                .ok_or_else(|| String::from("ready fixture omitted current fence"))?
                .created_at_millis
        )
    );
    assert!(!String::from_utf8_lossy(&body).contains("fence_token"));
    let secret = ready
        .service
        .recovery_lease
        .as_ref()
        .ok_or_else(|| String::from("ready fixture omitted its lease"))?
        .fence_token
        .clone();
    assert!(!String::from_utf8_lossy(&body).contains(&secret));

    let (retry_status, retry_body) = ready.service.handle_request(&request);
    assert_eq!(retry_status, 200);
    let retry = response_value(&retry_body)?;
    assert_eq!(retry["payload"], response["payload"]);
    assert_eq!(ready.service.recovery_lease.as_ref().map(|lease| &lease.lease_id), Some(&owner_before.lease_id));
    let current_time = ready.service.recovery_now_millis();
    let store = ready
        .service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("ready fixture omitted recovery store"))?;
    let persisted_owner = store
        .current_continuation_owner(
            &ready.service.config.session_id,
            current_time,
        )
        .map_err(|error| error.to_string())?
        .owner
        .ok_or_else(|| String::from("adoption removed the live owner"))?;
    assert_eq!(persisted_owner, owner_before);
    let persisted_claim = store
        .lookup_continuation_owner_claim(&operation_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("adoption removed the durable claim"))?;
    assert_eq!(
        persisted_claim.request_digest,
        response["payload"]["claim"]["request_digest"]
    );
    assert_eq!(
        persisted_claim.claimed_at_millis,
        response["payload"]["claim"]["claimed_at_millis"]
            .as_u64()
            .ok_or_else(|| String::from("adoption returned an invalid claim timestamp"))?
    );
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

#[test]
fn adoption_refuses_missing_claim_expired_owner_missing_grant_and_old_boot() -> Result<(), String> {
    let mut missing_claim = ready_service()?;
    let operation_id = Uuid::new_v4().to_string();
    let adopt_request = adoption_request(
        &operation_id,
        &missing_claim.owner,
        &missing_claim.service.config.caller_id,
    )?;
    assert_eq!(
        missing_claim.service.handle_request(&adopt_request).0,
        404
    );
    drop(missing_claim.service);
    remove_database(&missing_claim.path);

    let mut expired = ready_service()?;
    let lease = expired
        .service
        .recovery_lease
        .as_ref()
        .ok_or_else(|| String::from("ready fixture omitted its lease"))?;
    expired
        .service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("ready fixture omitted recovery store"))?
        .expire_leases(lease.expires_at_millis)
        .map_err(|error| error.to_string())?;
    let expired_adopt_request = adoption_request(
        &Uuid::new_v4().to_string(),
        &expired.owner,
        &expired.service.config.caller_id,
    )?;
    assert_eq!(
        expired.service.handle_request(&expired_adopt_request).0,
        410
    );
    drop(expired.service);
    remove_database(&expired.path);

    let mut missing_grant = ready_service()?;
    let operation_id = Uuid::new_v4().to_string();
    let claim = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&missing_grant.owner).map_err(|error| error.to_string())?,
        }),
        &missing_grant.service.config.caller_id,
    )?;
    assert_eq!(missing_grant.service.handle_request(&claim).0, 200);
    missing_grant.service.recovery_host_grant = None;
    let adopt_request = adoption_request(
        &operation_id,
        &missing_grant.owner,
        &missing_grant.service.config.caller_id,
    )?;
    assert_eq!(
        missing_grant.service.handle_request(&adopt_request).0,
        503
    );
    drop(missing_grant.service);
    remove_database(&missing_grant.path);

    let mut old_boot = ready_service()?;
    let operation_id = Uuid::new_v4().to_string();
    let claim = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&old_boot.owner).map_err(|error| error.to_string())?,
        }),
        &old_boot.service.config.caller_id,
    )?;
    assert_eq!(old_boot.service.handle_request(&claim).0, 200);
    old_boot
        .service
        .recovery_boot
        .as_mut()
        .ok_or_else(|| String::from("ready fixture omitted current boot"))?
        .boot_id = Uuid::new_v4().to_string();
    let adopt_request = adoption_request(
        &operation_id,
        &old_boot.owner,
        &old_boot.service.config.caller_id,
    )?;
    assert_eq!(old_boot.service.handle_request(&adopt_request).0, 503);
    drop(old_boot.service);
    remove_database(&old_boot.path);
    Ok(())
}

#[test]
fn adoption_rejects_a_retained_claim_for_a_different_current_lease() -> Result<(), String> {
    let mut ready = ready_service()?;
    let operation_id = Uuid::new_v4().to_string();
    let old_owner = ready.owner.clone();
    let claim = request(
        ContinuationOwnerKind::Claim,
        json!({
            "operation_id": operation_id,
            "expected_owner": serde_json::to_value(&old_owner).map_err(|error| error.to_string())?,
        }),
        &ready.service.config.caller_id,
    )?;
    assert_eq!(ready.service.handle_request(&claim).0, 200);
    replace_ready_lease(&mut ready)?;
    assert_ne!(ready.owner.lease_epoch, old_owner.lease_epoch);

    let request = adoption_request(
        &operation_id,
        &ready.owner,
        &ready.service.config.caller_id,
    )?;
    assert_eq!(ready.service.handle_request(&request).0, 409);
    let persisted = ready
        .service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("ready fixture omitted recovery store"))?
        .lookup_continuation_owner_claim(&operation_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("foreign claim disappeared"))?;
    assert_eq!(persisted.owner, old_owner);
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

#[test]
fn adoption_auth_and_closed_frame_rejections_precede_owner_lookup() -> Result<(), String> {
    let mut ready = ready_service()?;
    let operation_id = Uuid::new_v4().to_string();
    let mut wrong_capability = adoption_request(
        &operation_id,
        &ready.owner,
        &ready.service.config.caller_id,
    )?;
    wrong_capability.headers.insert(
        "x-sts2-recovery-capability".to_owned(),
        "continuation_owner_read".to_owned(),
    );
    assert_eq!(ready.service.handle_request(&wrong_capability).0, 403);

    let wrong_principal = adoption_request(&operation_id, &ready.owner, "foreign-caller")?;
    assert_eq!(ready.service.handle_request(&wrong_principal).0, 403);

    ready.service.config.auth_policy =
        AuthPolicy::test_with_previous("recovery-token", None, None, "read")?;
    let control_scope_missing =
        adoption_request(&operation_id, &ready.owner, &ready.service.config.caller_id)?;
    assert_eq!(ready.service.handle_request(&control_scope_missing).0, 403);
    ready.service.config.auth_policy =
        AuthPolicy::test_with_previous("recovery-token", None, None, "read,control")?;

    let mut duplicate = adoption_request(
        &operation_id,
        &ready.owner,
        &ready.service.config.caller_id,
    )?;
    let body = String::from_utf8(duplicate.body).map_err(|error| error.to_string())?;
    duplicate.body =
        format!(r#"{{"kind":"owner_adopt_request",{}"#, &body[1..]).into_bytes();
    assert_eq!(ready.service.handle_request(&duplicate).0, 400);

    let mut oversized = adoption_request(
        &operation_id,
        &ready.owner,
        &ready.service.config.caller_id,
    )?;
    oversized.body = vec![b' '; super::super::super::continuation_owner_adopt::MAX_FRAME_BYTES + 1];
    assert_eq!(ready.service.handle_request(&oversized).0, 413);
    drop(ready.service);
    remove_database(&ready.path);
    Ok(())
}

include!("service_recovery_owner_adopt_lease_fixture.rs");
