// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::{
    GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryLeaseRequest,
    RecoveryOperationIntent, RecoveryOperationState, RecoveryUncertaintyReason,
    canonical_json_digest,
};
use uuid::Uuid;

use super::super::RuntimeV3GameplayRoute;
use super::test_service;

const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
const STATE: &str = "00000000-0000-4000-8000-000000000003";
const OPERATION: &str = "00000000-0000-4000-8000-000000000004";

#[test]
fn unknown_duplicate_replays_original_binding_before_missing_cache() -> Result<(), String> {
    let path = std::env::temp_dir().join(format!(
        "sts2-gateway-v3-duplicate-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    let mut service = test_service()?;
    service.config.instance_id = INSTANCE.to_owned();
    let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let boot = store
        .start_boot(
            DEPLOYMENT,
            INSTANCE,
            service.config.recovery_release.clone(),
            1_000,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, 1_001)
        .map_err(|error| error.to_string())?;
    let lease = store
        .acquire_lease(RecoveryLeaseRequest {
            deployment_id: DEPLOYMENT.to_owned(),
            instance_id: INSTANCE.to_owned(),
            instance_incarnation: boot.instance_incarnation.clone(),
            boot_id: boot.boot_id.clone(),
            authority_generation: boot.authority_generation,
            host_fence_id: fence.host_fence_id.clone(),
            host_fence_generation: fence.fence_generation,
            caller_id: service.config.caller_id.clone(),
            session_id: service.config.session_id.clone(),
            now_millis: 1_002,
            ttl_seconds: 30,
            renewal_interval_seconds: 10,
        })
        .map_err(|error| error.to_string())?;
    let installation_id = Uuid::new_v4().to_string();
    let grant_digest = "a".repeat(64);
    store
        .prepare_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            &fence.host_fence_id,
            fence.fence_generation,
            1_003,
        )
        .map_err(|error| error.to_string())?;
    store
        .complete_host_lease_install(
            &lease.lease_id,
            &installation_id,
            &grant_digest,
            1,
            &Uuid::new_v4().to_string(),
            1_004,
        )
        .map_err(|error| error.to_string())?;
    let action = br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#;
    let operation = RecoveryOperationIntent {
        operation_id: OPERATION.to_owned(),
        deployment_id: lease.deployment_id.clone(),
        instance_id: lease.instance_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        boot_id: lease.boot_id.clone(),
        authority_generation: lease.authority_generation,
        lease_id: lease.lease_id.clone(),
        lease_epoch: lease.lease_epoch,
        schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
        canonical_json: action.to_vec(),
        payload_digest: canonical_json_digest(action).map_err(|error| error.to_string())?,
        expected_state_id: STATE.to_owned(),
        expected_generation: 0,
        catalog_digest: "b".repeat(64),
        now_millis: 1_003,
    };
    let proof = lease.proof();
    match store
        .record_intent(&proof, operation)
        .map_err(|error| error.to_string())?
    {
        RecoveryIntentResult::Created(_) => {}
        RecoveryIntentResult::Duplicate(_) => return Err(String::from("unexpected duplicate")),
    }
    store
        .mark_dispatched(&proof, INSTANCE, OPERATION, 1_004)
        .map_err(|error| error.to_string())?;
    store
        .record_outcome(
            INSTANCE,
            OPERATION,
            RecoveryOperationState::Unknown,
            None,
            None,
            None,
            Some(RecoveryUncertaintyReason::Timeout),
            1_005,
        )
        .map_err(|error| error.to_string())?;
    service.recovery = Some(store);
    service.recovery_boot = Some(boot);
    service.recovery_fence = Some(fence);
    service.recovery_lease = Some(lease);

    let mut request = super::authenticated_request("/v3/instances/unused/action");
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), String::from("corr"));
    let envelope = json!({
        "operation_id": OPERATION,
        "state_id": STATE,
        "generation": 0,
        "action": {"action_id": "action-end-turn", "action": {"kind": "end_turn"}}
    });
    let (status, body) = service.recovery_v3_dispatch(
        &request,
        RuntimeV3GameplayRoute::DispatchAction,
        envelope,
    );
    assert_eq!(status, 503);
    assert!(
        String::from_utf8(body)
            .map_err(|error| error.to_string())?
            .contains("UNKNOWN")
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
    Ok(())
}
