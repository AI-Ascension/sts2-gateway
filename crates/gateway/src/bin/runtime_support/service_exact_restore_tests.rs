// SPDX-License-Identifier: MIT

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{
    GatewayRecoveryStore, RecoveryBootState, RecoveryLeaseRequest, RecoveryReleaseSet, sha256_hex,
};
use uuid::Uuid;

use super::super::super::super::auth::AuthPolicy;
use super::super::super::super::http::{HttpRequest, read_request, write_response};
use super::super::super::test_support::{authenticated_request, test_service};
use super::super::super::{HostLeaseGrant, RuntimeService};
use super::{CONTRACT, MAX_FRAME_BYTES, NEUTRAL_CONTRACT, NEUTRAL_SCHEMA_DIGEST, Route};

const NEUTRAL_ARTIFACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/exact-restore-v1"
);
const WRAPPER_ARTIFACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/exact-restore-gateway-v1"
);
const WRAPPER_SCHEMA_DIGEST: &str =
    "0b181dc30524c8b14dea73e490da55538f2d57fe87bf58ed9fe33223406a7d89";
const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000011";

struct ReadyService {
    service: RuntimeService,
    owner: Value,
    store_path: PathBuf,
}

fn ready_service() -> Result<ReadyService, String> {
    let mut service = test_service()?;
    service.config.auth_policy =
        AuthPolicy::test_with_previous("recovery-token", None, None, "control")?;
    service.config.instance_id = Uuid::new_v4().to_string();
    service.config.caller_id = Uuid::new_v4().to_string();
    service.config.session_id = format!("session-{}", Uuid::new_v4());
    service.config.recovery_deployment_id = Some(DEPLOYMENT.to_owned());

    let store_path = std::env::temp_dir().join(format!(
        "sts2-gateway-exact-restore-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut store = GatewayRecoveryStore::open(&store_path).map_err(|error| error.to_string())?;
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
    let grant = super::super::super::host_lease_helpers::grant_value(
        &boot,
        &fence,
        &lease,
        &service.config.caller_id,
        &service.config.session_id,
    );
    let grant_digest = super::super::super::super::host_lease_control::grant_digest(&grant)
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
    let owner = serde_json::to_value(
        store
            .current_continuation_owner(&service.config.session_id, now + 5)
            .map_err(|error| error.to_string())?
            .owner
            .ok_or("expected current owner")?,
    )
    .map_err(|error| error.to_string())?;
    service.recovery = Some(store);
    service.recovery_boot = Some(boot);
    service.recovery_fence = Some(fence);
    service.recovery_lease = Some(lease.clone());
    service.recovery_host_grant = Some(HostLeaseGrant {
        installation_id,
        grant_digest,
        grant,
    });
    service.lease_active = true;
    service.recovery_lease_deadline = Some(Instant::now() + Duration::from_secs(30));
    service.recovery_lease_deadline_lease_id = Some(lease.lease_id);
    if !service
        .active_host_grant_matches(
            service
                .recovery_lease
                .as_ref()
                .ok_or("expected active lease")?,
        )
        .map_err(|error| error.to_string())?
    {
        return Err(String::from("ready fixture host grant is not canonical"));
    }
    Ok(ReadyService {
        service,
        owner,
        store_path,
    })
}

fn request_frame(
    owner: &Value,
    kind: &str,
    message_id: &str,
    correlation_id: &str,
) -> Result<Value, String> {
    let frames: Value = serde_json::from_slice(
        &std::fs::read(PathBuf::from(NEUTRAL_ARTIFACT).join("golden/frames.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut frame = frames["frames"]
        .as_array()
        .and_then(|frames| frames.iter().find(|frame| frame["kind"] == kind))
        .cloned()
        .ok_or_else(|| format!("missing golden request kind {kind}"))?;
    frame["message_id"] = json!(message_id);
    frame["correlation_id"] = json!(correlation_id);
    frame["payload"]["expected_owner"] = owner.clone();
    Ok(frame)
}

fn wrapper_request(frame: Value, principal: &str) -> Result<Vec<u8>, String> {
    let wrapper = json!({
        "contract": CONTRACT,
        "schema_digest": WRAPPER_SCHEMA_DIGEST,
        "message_id": frame["message_id"],
        "correlation_id": frame["correlation_id"],
        "actor": {"principal_id": principal, "role": "harness"},
        "auth": {"principal_id": principal, "capability": "exact_restore", "proof": null},
        "kind": "exact_restore_request",
        "payload": {"frame": frame},
    });
    serde_json::to_vec(&wrapper).map_err(|error| error.to_string())
}

fn authenticated_exact_request(
    service: &RuntimeService,
    path: &str,
    body: Vec<u8>,
    correlation: &str,
) -> HttpRequest {
    let mut request = authenticated_request(path);
    request.method = "POST".to_owned();
    request.headers.insert(
        "authorization".to_owned(),
        "Bearer recovery-token".to_owned(),
    );
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.headers.insert(
        "x-sts2-recovery-capability".to_owned(),
        "exact_restore".to_owned(),
    );
    request
        .headers
        .insert("x-sts2-correlation-id".to_owned(), correlation.to_owned());
    for (header, value) in [
        ("x-sts2-instance-id", service.config.instance_id.as_str()),
        ("x-sts2-caller-id", service.config.caller_id.as_str()),
        ("x-sts2-session-id", service.config.session_id.as_str()),
        ("x-mcp-session-id", service.config.mcp_session_id.as_str()),
    ] {
        request.headers.insert(header.to_owned(), value.to_owned());
    }
    if let Some(lease) = service.recovery_lease.as_ref() {
        request
            .headers
            .insert("x-sts2-lease-id".to_owned(), lease.lease_id.clone());
        request.headers.insert(
            "x-sts2-lease-epoch".to_owned(),
            lease.lease_epoch.to_string(),
        );
    }
    request.body = body;
    request
}

fn error_response(request: &Value, outcome: &str, error_code: &str) -> Result<Vec<u8>, String> {
    let neutral = json!({
        "contract": NEUTRAL_CONTRACT,
        "schema_digest": NEUTRAL_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": request["message_id"],
        "kind": "exact_restore_error_response",
        "payload": {
            "operation_id": request["payload"]["operation_id"],
            "expected_owner": request["payload"]["expected_owner"],
            "request_digest": format!(
                "sha256:{}",
                sha256_hex(&serde_json::to_vec(request).map_err(|error| error.to_string())?)
            ),
            "outcome": outcome,
            "error_code": error_code,
            "host_effect": if outcome == "UNAVAILABLE" {
                "may_have_started"
            } else {
                "not_started"
            },
        }
    });
    serde_json::to_vec(&neutral).map_err(|error| error.to_string())
}

fn cleanup(path: &PathBuf) {
    for candidate in [
        path.to_owned(),
        path.with_extension("db-shm"),
        path.with_extension("db-wal"),
        path.with_extension("gateway-recovery.lock"),
    ] {
        let _ = std::fs::remove_file(candidate);
    }
}

#[test]
fn routes_are_fixed_post_only_and_bound_to_neutral_phase_kinds() {
    let paths = [
        (Route::Begin, "/v1/exact-restore/begin"),
        (Route::Chunk, "/v1/exact-restore/chunk"),
        (Route::Finish, "/v1/exact-restore/finish"),
        (Route::Commit, "/v1/exact-restore/commit"),
        (Route::Lookup, "/v1/exact-restore/lookup"),
    ];
    for (route, path) in paths {
        assert_eq!(Route::parse("POST", path), Some(route));
        assert_eq!(Route::parse("GET", path), None);
        assert_eq!(Route::parse("POST", &format!("{path}/extra")), None);
    }
    assert_eq!(Route::parse("POST", "/v1/exact-restore/other"), None);
}

#[test]
fn foreign_owner_wrong_capability_and_malformed_frame_do_not_forward() -> Result<(), String> {
    let mut ready = ready_service()?;
    let trap = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    ready.service.config.mod_address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();

    let message_id = Uuid::new_v4().to_string();
    let correlation_id = Uuid::new_v4().to_string();
    let mut foreign_owner = ready.owner.clone();
    let next_epoch = ready.owner["lease_epoch"]
        .as_u64()
        .ok_or("missing lease epoch")?
        + 1;
    foreign_owner["lease_epoch"] = json!(next_epoch);
    let frame = request_frame(
        &foreign_owner,
        "exact_restore_begin_request",
        &message_id,
        &correlation_id,
    )?;
    let body = wrapper_request(frame, &ready.service.config.caller_id)?;
    let mut request =
        authenticated_exact_request(&ready.service, Route::Begin.path(), body, &correlation_id);
    assert_eq!(ready.service.handle_request(&request).0, 409);

    request.body = wrapper_request(
        request_frame(
            &ready.owner,
            "exact_restore_begin_request",
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
        )?,
        &ready.service.config.caller_id,
    )?;
    request
        .headers
        .insert("x-sts2-recovery-capability".to_owned(), "wrong".to_owned());
    assert_eq!(ready.service.handle_request(&request).0, 403);

    request.headers.insert(
        "x-sts2-recovery-capability".to_owned(),
        "exact_restore".to_owned(),
    );
    request.body = b"{\"contract\":\"bad\",\"contract\":\"duplicate\"}".to_vec();
    assert_eq!(ready.service.handle_request(&request).0, 400);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    cleanup(&ready.store_path);
    Ok(())
}

#[test]
fn missing_installed_host_grant_refuses_before_native_forwarding() -> Result<(), String> {
    let mut ready = ready_service()?;
    let trap = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    trap.set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    ready.service.config.mod_address = trap
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    ready.service.recovery_host_grant = None;

    let message_id = Uuid::new_v4().to_string();
    let correlation_id = Uuid::new_v4().to_string();
    let frame = request_frame(
        &ready.owner,
        "exact_restore_begin_request",
        &message_id,
        &correlation_id,
    )?;
    let request = authenticated_exact_request(
        &ready.service,
        Route::Begin.path(),
        wrapper_request(frame, &ready.service.config.caller_id)?,
        &correlation_id,
    );
    assert_eq!(ready.service.handle_request(&request).0, 503);
    assert!(matches!(
        trap.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    cleanup(&ready.store_path);
    Ok(())
}

#[test]
fn frozen_consumer_artifacts_and_schema_pins_match_gateway_source() -> Result<(), String> {
    let wrapper_root = PathBuf::from(WRAPPER_ARTIFACT);
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(wrapper_root.join("manifest.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let schema =
        std::fs::read(wrapper_root.join("schema.json")).map_err(|error| error.to_string())?;
    assert_eq!(manifest["contract"], CONTRACT);
    assert_eq!(manifest["schema_digest"], WRAPPER_SCHEMA_DIGEST);
    assert_eq!(sha256_hex(&schema), WRAPPER_SCHEMA_DIGEST);

    let neutral_manifest: Value = serde_json::from_slice(
        &std::fs::read(PathBuf::from(NEUTRAL_ARTIFACT).join("manifest.json"))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(neutral_manifest["protocol_version"], "exact-restore-v1");
    assert_eq!(neutral_manifest["schema_digest"], NEUTRAL_SCHEMA_DIGEST);
    Ok(())
}

#[path = "service_exact_restore_production_tests.rs"]
mod production_tests;
