// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_gateway::{
    GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryLeaseRequest,
    RecoveryOperationIntent, RecoveryOperationState, RecoveryUncertaintyReason,
    canonical_json_digest,
};
use uuid::Uuid;

use super::super::RuntimeV3GameplayRoute;
use super::{authenticated_request, test_service};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

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

#[test]
fn runtime_v3_non_uuid_correlation_uses_uuid_host_frames() -> Result<(), String> {
    run_runtime_v3_translation(true)
}

#[test]
fn runtime_v3_settled_translation_query_failure_is_unknown() -> Result<(), String> {
    run_runtime_v3_translation(false)
}

fn run_runtime_v3_translation(state_ok: bool) -> Result<(), String> {
    let (mut service, lease, path) = super::super::runtime_v3_catalog_tests::
        recovery_service_with_caller("00000000-0000-4000-8000-000000000008")?;
    let mut dispatch = super::super::runtime_v3_catalog_tests::dispatch_envelope(
        &service,
        &lease,
        "3",
    )?;
    super::super::runtime_v3_catalog_tests::capture_old_catalog(
        &mut service,
        &lease,
        &dispatch,
    )?;
    dispatch["correlation_id"] = "3".into();
    let mut request = authenticated_request("/v3/instances/unused/action");
    request.method = String::from("POST");
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), String::from("3"));
    request.body = serde_json::to_vec(&dispatch).map_err(|error| error.to_string())?;
    let operation_id = dispatch["operation_id"].clone();
    service.recovery_test_bootstrap_secret = Some(vec![b'a'; 32]);

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let fence_id = service
        .recovery_fence
        .as_ref()
        .map(|fence| fence.host_fence_id.clone())
        .ok_or_else(|| String::from("recovery fence missing"))?;
    let worker = thread::spawn(move || -> Result<Vec<String>, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut correlations = Vec::new();
        let mut intent_operation = None;
        for (index, kind) in [
            super::super::super::recovery_frame::RecoveryKind::OperationIntent,
            super::super::super::recovery_frame::RecoveryKind::OperationDispatch,
        ]
        .into_iter()
        .enumerate()
        {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(format!("accept {index}: {error}")),
                }
            };
            let forwarded = super::super::super::http::read_request(&mut stream)
                .map_err(|error| format!("{error:?}"))?;
            let value: Value =
                serde_json::from_slice(&forwarded.body).map_err(|error| error.to_string())?;
            let correlation = value["correlation_id"]
                .as_str()
                .ok_or_else(|| String::from("host correlation missing"))?
                .to_owned();
            let parsed = uuid::Uuid::parse_str(&correlation)
                .map_err(|error| format!("host correlation is not UUID: {error}"))?;
            if parsed.get_version_num() != 4 {
                return Err(String::from("host correlation is not UUIDv4"));
            }
            correlations.push(correlation.clone());
            let operation = if kind
                == super::super::super::recovery_frame::RecoveryKind::OperationIntent
            {
                intent_operation = Some(value["payload"]["operation"].clone());
                value["payload"]["operation"].clone()
            } else {
                let mut operation = intent_operation
                    .clone()
                    .ok_or_else(|| String::from("intent operation missing"))?;
                let lease = value["payload"]["lease"].clone();
                operation["state"] = "SETTLED".into();
                operation["ticket"] = json!({
                    "ticket_id": "00000000-0000-4000-8000-000000000004",
                    "operation_id": operation["operation_id"],
                    "payload_digest": operation["payload_digest"],
                    "boot_id": lease["boot_id"],
                    "instance_incarnation": lease["instance_incarnation"],
                    "lease_epoch": lease["lease_epoch"],
                    "host_fence_id": fence_id,
                    "state": "SETTLED",
                    "issued_at": "2026-09-17T00:00:00Z",
                    "expires_at": "2026-09-17T00:05:00Z",
                });
                operation["witness"] = json!({
                    "witness_id": "00000000-0000-4000-8000-000000000005",
                    "operation_id": operation["operation_id"],
                    "payload_digest": operation["payload_digest"],
                    "boot_id": lease["boot_id"],
                    "instance_incarnation": lease["instance_incarnation"],
                    "host_fence_id": fence_id,
                    "source": "host_game_thread",
                    "state_id": STATE,
                    "generation": 2,
                    "effect_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "observed_at": "2026-09-17T00:00:01Z",
                });
                operation["uncertainty_reason"] = Value::Null;
                operation["created_at"] = "2026-09-17T00:00:00Z".into();
                operation["updated_at"] = "2026-09-17T00:00:01Z".into();
                operation
            };
            let result = if kind
                == super::super::super::recovery_frame::RecoveryKind::OperationIntent
            {
                super::super::super::recovery_frame::response_result(
                    "INTENT_RECORDED",
                    false,
                    None,
                )
            } else {
                super::super::super::recovery_frame::response_result("SETTLED", false, None)
            };
            let body = json!({
                "result": result,
                "operation": operation,
            });
            let response = super::super::super::recovery_frame::response_frame(
                kind,
                &correlation,
                "00000000-0000-4000-8000-000000000009",
                body,
            );
            super::super::super::http::write_response(&mut stream, 200, &response)
                .map_err(|error| error.to_string())?;
        }
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error)
                    if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(format!("state accept: {error}")),
            }
        };
        let state_request = super::super::super::http::read_request(&mut stream)
            .map_err(|error| format!("{error:?}"))?;
        if state_request.method != "GET" || state_request.path != "/api/v3/runtime/state" {
            return Err(format!(
                "unexpected state request {} {}",
                state_request.method, state_request.path
            ));
        }
        let state_request: Value =
            serde_json::from_slice(&state_request.body).map_err(|error| error.to_string())?;
        if !state_ok {
            super::super::super::http::write_response(&mut stream, 503, b"{}")
                .map_err(|error| error.to_string())?;
            return Ok(correlations);
        }
        let mut state = super::super::runtime_v3_catalog_tests::fixture("state-response.json")?;
        for field in [
            "correlation_id",
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
        ] {
            state[field] = state_request[field].clone();
        }
        state["generation"] = 2.into();
        state["state_id"] = STATE.into();
        state["observation"]["state_id"] = STATE.into();
        state["observation"]["generation"] = 2.into();
        state["legal_actions"] = json!([]);
        super::super::super::http::write_response(
            &mut stream,
            200,
            &serde_json::to_vec(&state).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(correlations)
    });
    service.config.mod_address = address;
    let (status, body) = service.runtime_v3_recovery_dispatch(
        &request,
        RuntimeV3GameplayRoute::DispatchAction,
        dispatch,
    );
    let worker_result = worker
        .join()
        .map_err(|_| String::from("host worker panicked"))?;
    let correlations = worker_result.map_err(|error| {
        format!(
            "{error}; gateway status={status} body={}",
            String::from_utf8_lossy(&body)
        )
    })?;
    let response: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(response["correlation_id"], "3");
    if state_ok {
        assert_eq!(status, 200, "body={}", String::from_utf8_lossy(&body));
        assert_eq!(response["kind"], "dispatch_action_response");
        assert_eq!(response["status"], "settled");
        assert_eq!(response["operation_id"], operation_id);
        assert_eq!(
            response["transition"]["effect_kind"],
            "recovery_operation_settled"
        );
    } else {
        assert_eq!(status, 503);
        assert_eq!(response["kind"], "dispatch_action_response");
        assert_eq!(response["status"], "unknown");
        assert_eq!(response["error_code"], "settlement_observation_unavailable");
    }
    assert_eq!(correlations.len(), 2);
    assert_ne!(correlations[0], "3");
    assert_ne!(correlations[0], correlations[1]);
    super::super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
