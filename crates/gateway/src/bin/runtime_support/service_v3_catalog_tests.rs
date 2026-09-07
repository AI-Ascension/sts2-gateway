// SPDX-License-Identifier: MIT

use super::recovery_catalog_tests::refresh_same_generation_catalog;
use super::test_support::*;
use super::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sts2_gateway::{GatewayRecoveryStore, RecoveryLease, RecoveryLeaseRequest};

pub(super) const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
pub(super) const OLD_STATE: &str = "00000000-0000-4000-8000-000000000003";
pub(super) const DISPATCH_OPERATION: &str = "00000000-0000-4000-8000-000000000004";
const NEW_STATE: &str = "00000000-0000-4000-8000-000000000005";
const WAIT_OPERATION: &str = "00000000-0000-4000-8000-000000000006";
const RECOVER_OPERATION: &str = "00000000-0000-4000-8000-000000000007";

pub(super) fn fixture(name: &str) -> Result<Value, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/runtime-v3-gameplay/golden")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

pub(super) fn recovery_service() -> Result<(RuntimeService, RecoveryLease, PathBuf), String> {
    let mut service = test_service()?;
    service.config.instance_id = INSTANCE.to_owned();
    service.config.recovery_deployment_id = Some(DEPLOYMENT.to_owned());
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-gateway-v3-forwarding-{}-{suffix}.db",
        std::process::id()
    ));
    let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
    let now = service.recovery_now_millis();
    let boot = store
        .start_boot(
            DEPLOYMENT,
            INSTANCE,
            service.config.recovery_release.clone(),
            now,
        )
        .map_err(|error| error.to_string())?;
    let fence = store
        .complete_host_fence(&boot, now.saturating_add(1))
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
            now_millis: now.saturating_add(2),
            ttl_seconds: service.config.recovery_ttl_seconds,
            renewal_interval_seconds: service.config.recovery_renewal_interval_seconds,
        })
        .map_err(|error| error.to_string())?;
    service.recovery = Some(store);
    service.recovery_boot = Some(boot);
    service.recovery_fence = Some(fence);
    service.recovery_lease_deadline =
        Some(std::time::Instant::now() + Duration::from_secs(lease.ttl_seconds));
    service.recovery_lease = Some(lease.clone());
    service.lease_active = true;
    Ok((service, lease, path))
}

pub(super) fn cleanup(service: RuntimeService, path: &Path) {
    drop(service);
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
}

pub(super) fn bind(
    value: &mut Value,
    service: &RuntimeService,
    lease: &RecoveryLease,
    correlation: &str,
) {
    value["correlation_id"] = correlation.into();
    value["instance_id"] = service.config.instance_id.clone().into();
    value["session_id"] = service.config.session_id.clone().into();
    value["lease_id"] = lease.lease_id.clone().into();
    value["lease_epoch"] = lease.lease_epoch.into();
}

pub(super) fn runtime_request(
    service: &RuntimeService,
    lease: &RecoveryLease,
    suffix: &str,
    value: Value,
) -> Result<HttpRequest, String> {
    let mut request = authenticated_request(&format!(
        "/v3/instances/{}/{suffix}",
        service.config.instance_id
    ));
    request.method = if matches!(suffix, "action" | "wait" | "recover") {
        String::from("POST")
    } else {
        String::from("GET")
    };
    request.headers.insert(
        String::from("x-sts2-instance-id"),
        service.config.instance_id.clone(),
    );
    request
        .headers
        .insert(String::from("x-sts2-lease-id"), lease.lease_id.clone());
    request.headers.insert(
        String::from("x-sts2-lease-epoch"),
        lease.lease_epoch.to_string(),
    );
    let correlation = value["correlation_id"]
        .as_str()
        .ok_or_else(|| String::from("correlation missing from envelope"))?;
    request.headers.insert(
        String::from("x-sts2-correlation-id"),
        correlation.to_owned(),
    );
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    Ok(request)
}

pub(super) fn dispatch_envelope(
    service: &RuntimeService,
    lease: &RecoveryLease,
    correlation: &str,
) -> Result<Value, String> {
    let mut value = fixture("dispatch-action-request.json")?;
    value["operation_id"] = DISPATCH_OPERATION.into();
    value["state_id"] = OLD_STATE.into();
    value["generation"] = 1.into();
    bind(&mut value, service, lease, correlation);
    Ok(value)
}

pub(super) fn capture_old_catalog(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
    dispatch: &Value,
) -> Result<(), String> {
    let body = json!({
        "instance_id": service.config.instance_id,
        "session_id": service.config.session_id,
        "lease_id": lease.lease_id,
        "lease_epoch": lease.lease_epoch,
        "kind": "legal_actions_response",
        "state_id": OLD_STATE,
        "generation": 1,
        "legal_actions": [dispatch["action"]],
    });
    let bytes = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    if service.capture_recovery_catalog(lease, 200, &bytes) {
        Ok(())
    } else {
        Err(String::from("old catalog capture failed"))
    }
}

fn settled_response(
    request: &Value,
    kind: &str,
    operation_id: &str,
    wait_outcome: Option<&str>,
) -> Result<Value, String> {
    let mut response = fixture("state-response.json")?;
    response["kind"] = kind.into();
    response["correlation_id"] = request["correlation_id"].clone();
    response["instance_id"] = request["instance_id"].clone();
    response["session_id"] = request["session_id"].clone();
    response["lease_id"] = request["lease_id"].clone();
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["generation"] = 2.into();
    response["state_id"] = NEW_STATE.into();
    response["operation_id"] = operation_id.into();
    response["observation"]["state_id"] = NEW_STATE.into();
    response["observation"]["generation"] = 2.into();
    response["status"] = "settled".into();
    response["transition"] = json!({
        "from_generation": 1,
        "to_generation": 2,
        "state_id": NEW_STATE,
        "effect_kind": "synthetic.settled"
    });
    response["wait_outcome"] = wait_outcome.map_or(Value::Null, Value::from);
    response["recovery"] = Value::Null;
    Ok(response)
}

fn unknown_wait_response(request: &Value) -> Result<Value, String> {
    let mut response = fixture("state-response.json")?;
    response["kind"] = "wait_response".into();
    response["correlation_id"] = request["correlation_id"].clone();
    response["instance_id"] = request["instance_id"].clone();
    response["session_id"] = request["session_id"].clone();
    response["lease_id"] = request["lease_id"].clone();
    response["lease_epoch"] = request["lease_epoch"].clone();
    response["generation"] = 1.into();
    response["state_id"] = Value::Null;
    response["operation_id"] = WAIT_OPERATION.into();
    response["observation"] = Value::Null;
    response["legal_actions"] = Value::Null;
    response["status"] = "unknown".into();
    response["transition"] = Value::Null;
    response["error_code"] = "wait_timeout".into();
    response["wait_outcome"] = "timeout".into();
    response["recovery"] = Value::Null;
    Ok(response)
}

pub(super) fn forward_once(
    service: &mut RuntimeService,
    request: &HttpRequest,
    response_status: u16,
    response_body: &[u8],
) -> Result<(u16, Vec<u8>, HttpRequest), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let response_body = response_body.to_vec();
    let worker = thread::spawn(move || -> Result<HttpRequest, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(format!("accept: {error}")),
            }
        };
        let forwarded =
            read_request(&mut stream).map_err(|status| format!("read status: {status}"))?;
        write_response(&mut stream, response_status, &response_body)
            .map_err(|error| format!("write: {error}"))?;
        Ok(forwarded)
    });
    service.config.mod_address = address.to_string();
    let (status, body) = service.handle_request(request);
    let forwarded = match worker.join() {
        Ok(Ok(forwarded)) => forwarded,
        Ok(Err(error)) => {
            return Err(format!(
                "downstream worker failed after gateway status {status}: {error}; body={}",
                String::from_utf8_lossy(&body)
            ));
        }
        Err(_) => {
            return Err(format!(
                "downstream worker panicked after gateway status {status}; body={}",
                String::from_utf8_lossy(&body)
            ));
        }
    };
    Ok((status, body, forwarded))
}

pub(super) fn json_body(body: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(body).map_err(|error| error.to_string())
}

#[test]
fn settled_wait_forward_invalidates_old_catalog_before_dispatch_admission() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "dispatch-after-wait")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;
    let mut wait = fixture("state-request.json")?;
    wait["kind"] = "wait_request".into();
    wait["operation_id"] = WAIT_OPERATION.into();
    wait["generation"] = 1.into();
    wait["wait_for_millis"] = 1.into();
    bind(&mut wait, &service, &lease, "wait-correlation");
    let response = settled_response(&wait, "wait_response", WAIT_OPERATION, Some("successor"))?;
    let wait_request = runtime_request(&service, &lease, "wait", wait.clone())?;
    let response_body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    let (status, body, forwarded) = forward_once(&mut service, &wait_request, 200, &response_body)?;
    assert_eq!(status, 200);
    assert_eq!(body, response_body);
    assert_eq!(forwarded.path, "/api/v3/runtime/wait");

    let dispatch_request = runtime_request(&service, &lease, "action", dispatch)?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 409);
    assert_eq!(
        json_body(&body)?["error_code"],
        "recovery_catalog_fresh_read_required"
    );
    cleanup(service, &path);
    Ok(())
}

#[test]
fn settled_recover_forward_invalidates_old_catalog_before_dispatch_admission() -> Result<(), String>
{
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "dispatch-after-recover")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;
    let mut recover = fixture("state-request.json")?;
    recover["kind"] = "recover_request".into();
    recover["generation"] = 1.into();
    recover["recovery"] = json!({"kind": "reconcile", "operation_id": RECOVER_OPERATION});
    bind(&mut recover, &service, &lease, "recover-correlation");
    let response = settled_response(&recover, "recover_response", RECOVER_OPERATION, None)?;
    let recover_request = runtime_request(&service, &lease, "recover", recover)?;
    let response_body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    let (status, body, forwarded) =
        forward_once(&mut service, &recover_request, 200, &response_body)?;
    assert_eq!(status, 200);
    assert_eq!(body, response_body);
    assert_eq!(forwarded.path, "/api/v3/runtime/recover");

    let dispatch_request = runtime_request(&service, &lease, "action", dispatch)?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 409);
    assert_eq!(
        json_body(&body)?["error_code"],
        "recovery_catalog_fresh_read_required"
    );
    cleanup(service, &path);
    Ok(())
}

#[test]
fn unknown_wait_forward_does_not_invent_freshness() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "dispatch-after-unknown")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;
    let mut wait = fixture("state-request.json")?;
    wait["kind"] = "wait_request".into();
    wait["operation_id"] = WAIT_OPERATION.into();
    wait["generation"] = 1.into();
    wait["wait_for_millis"] = 1.into();
    bind(&mut wait, &service, &lease, "unknown-wait-correlation");
    let response = unknown_wait_response(&wait)?;
    let wait_request = runtime_request(&service, &lease, "wait", wait)?;
    let response_body = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    let (status, body, _) = forward_once(&mut service, &wait_request, 503, &response_body)?;
    assert_eq!(status, 503);
    assert_eq!(body, response_body);

    let dispatch_request = runtime_request(&service, &lease, "action", dispatch)?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    assert_eq!(json_body(&body)?["payload"]["result"]["status"], "UNKNOWN");
    cleanup(service, &path);
    Ok(())
}

#[test]
fn durable_duplicate_replays_before_missing_catalog_admission() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "durable-duplicate-correlation")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;
    let dispatch_request = runtime_request(&service, &lease, "action", dispatch.clone())?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    assert_eq!(json_body(&body)?["payload"]["result"]["status"], "UNKNOWN");

    // The first dispatch consumed the catalog before the host outcome became
    // UNKNOWN. A delayed legal-actions response for that same boundary is
    // not a fresh authoritative read and must remain rejected.
    assert!(capture_old_catalog(&mut service, &lease, &dispatch).is_err());

    // A same-generation state read followed by a legal-actions read is fresh
    // catalog evidence, but it does not settle the durable UNKNOWN operation.
    refresh_same_generation_catalog(&mut service, &lease, &dispatch, "unknown-refresh")?;

    // The durable dispatch marker makes the old catalog unsafe even when the
    // host proof is unavailable. A different operation cannot be admitted,
    // while the exact original operation remains replayable.
    let mut new_dispatch = dispatch;
    new_dispatch["operation_id"] = "00000000-0000-4000-8000-000000000008".into();
    new_dispatch["correlation_id"] = "new-correlation".into();
    let new_request = runtime_request(&service, &lease, "action", new_dispatch)?;
    let (status, body) = service.handle_request(&new_request);
    assert_eq!(status, 413);
    assert_eq!(json_body(&body)?["error_code"], "recovery_bounds_exceeded");

    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    let replay = json_body(&body)?;
    assert_eq!(replay["payload"]["result"]["status"], "UNKNOWN");
    assert_eq!(
        replay["payload"]["operation"]["operation_id"],
        DISPATCH_OPERATION
    );
    cleanup(service, &path);
    Ok(())
}
