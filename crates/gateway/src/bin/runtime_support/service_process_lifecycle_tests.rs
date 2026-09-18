// SPDX-License-Identifier: MIT

//! Boundary tests for the attached process-lifecycle route surface.
//!
//! Every test drives the real HTTP entry point (`handle_request`) rather than calling the route
//! handler directly, so authorization, lease fencing, routing, and dispatch are exercised as they
//! ship. The process adapter is a deterministic synthetic port: these are source-component
//! evidence, not native game or OS-process evidence.

use super::*;
use sts2_gateway::{
    LaunchSpec, ProcessFault,
    ProcessHandle, ProcessIdentity, ProcessLaunch, ProcessPort, ProcessState, StopMode,
};
use std::sync::{Arc, Mutex, MutexGuard};

/// Reads the synthetic adapter's record without unwrapping a poisoned lock.
fn recorded(state: &Arc<Mutex<RecordingState>>) -> Result<MutexGuard<'_, RecordingState>, String> {
    state.lock().map_err(|error| error.to_string())
}

/// Deterministic approved-profile port. Records every start and returns an identity that matches
/// the resolved profile, so the coordinator's own identity verification runs for real.
struct RecordingProcess {
    state: Arc<Mutex<RecordingState>>,
}

#[derive(Default)]
struct RecordingState {
    starts: usize,
    stops: usize,
    next_handle: u64,
    identities: std::collections::BTreeMap<u64, ProcessIdentity>,
}

impl RecordingProcess {
    fn new() -> (Self, Arc<Mutex<RecordingState>>) {
        let state = Arc::new(Mutex::new(RecordingState::default()));
        (
            Self {
                state: Arc::clone(&state),
            },
            state,
        )
    }
}

impl ProcessPort for RecordingProcess {
    fn start(&mut self, _specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    fn start_with_profile(
        &mut self,
        specification: LaunchSpec,
        profile: sts2_gateway::LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProcessFault::StartRejected)?;
        state.next_handle = state.next_handle.saturating_add(1);
        state.starts += 1;
        let handle = ProcessHandle::new(state.next_handle);
        let identity = ProcessIdentity::new(
            specification.instance_id(),
            handle,
            1000 + handle.value(),
            5000 + handle.value(),
            profile.executable(),
            profile.user_data(),
        );
        state.identities.insert(handle.value(), identity.clone());
        Ok(ProcessLaunch::new(identity))
    }

    fn inspect(&mut self, _process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        Ok(ProcessState::Running)
    }

    fn stop(&mut self, _process: ProcessHandle, _mode: StopMode) -> Result<(), ProcessFault> {
        self.state
            .lock()
            .map_err(|_| ProcessFault::StopFailed)?
            .stops += 1;
        Ok(())
    }

    fn inspect_identity(&mut self, process: ProcessHandle) -> Result<ProcessIdentity, ProcessFault> {
        self.state
            .lock()
            .map_err(|_| ProcessFault::InspectionFailed)?
            .identities
            .get(&process.value())
            .cloned()
            .ok_or(ProcessFault::InspectionFailed)
    }

    fn recover_owned(
        &mut self,
        _instance_id: sts2_gateway::InstanceId,
        _profile: sts2_gateway::LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        Ok(None)
    }
}

/// Composes a service whose lifecycle surface is fully wired to a synthetic adapter.
///
/// This drives the real production composition path (`ProcessLifecycleRuntime::compose`), so the
/// catalog build, the durable SQLite store open, the capacity-budget validation, and the
/// coordinator construction are all exercised as they ship. Only the process port is synthetic.
fn lifecycle_service() -> Result<(RuntimeService, Arc<Mutex<RecordingState>>), String> {
    let mut service = test_support::test_service()?;
    let (process, state) = RecordingProcess::new();
    let deployment = service_process_lifecycle_config::ProcessLifecycleDeployment {
        profiles: process_lifecycle_fixtures::profiles()?,
        store_path: process_lifecycle_fixtures::store_path(),
        max_processes: 4,
        max_records: 16,
    };
    service.process_lifecycle = service_process_lifecycle::ProcessLifecycleRuntime::compose(
        deployment,
        Box::new(process) as Box<dyn ProcessPort + Send>,
    )?;
    assert!(
        service.process_lifecycle.is_ready(),
        "the composed runtime must be ready"
    );
    Ok((service, state))
}

fn lifecycle_request(method: &str, path: &str, body: &str) -> HttpRequest {
    let mut request = test_support::authenticated_request(path);
    request.method = String::from(method);
    if !body.is_empty() {
        request
            .headers
            .insert(String::from("content-type"), String::from("application/json"));
        request.body = body.as_bytes().to_vec();
    }
    request
}

fn operations_path() -> String {
    String::from("/v1/instances/instance-1/process-lifecycle/operations")
}

fn capability_path() -> String {
    String::from("/v1/instances/instance-1/process-lifecycle")
}

fn launch_body(operation_id: u64, profile_id: u64) -> String {
    format!(
        r#"{{"operation_id":{operation_id},"authority_epoch":1,"action":{{"kind":"launch_new","profile_id":{profile_id}}}}}"#
    )
}

#[test]
fn unconfigured_deployment_refuses_every_effect_before_any_port_call() -> Result<(), String> {
    let mut service = test_support::test_service()?;
    assert!(!service.process_lifecycle.is_ready());
    let response = service.handle_request(&lifecycle_request(
        "POST",
        &operations_path(),
        &launch_body(1, 7),
    ));
    assert_eq!(response.0, 503);
    assert!(
        String::from_utf8_lossy(&response.1).contains("process_lifecycle_unconfigured"),
        "unconfigured deployments must name the reason"
    );
    assert_eq!(
        service
            .handle_request(&lifecycle_request("GET", &capability_path(), ""))
            .0,
        503
    );
    Ok(())
}

#[test]
fn capability_lists_only_configured_profile_ids() -> Result<(), String> {
    let (mut service, _state) = lifecycle_service()?;
    let response = service.handle_request(&lifecycle_request("GET", &capability_path(), ""));
    assert_eq!(response.0, 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.1).map_err(|error| error.to_string())?;
    assert_eq!(body["profiles"], serde_json::json!([7, 9]));
    assert_eq!(body["available"], serde_json::json!(true));
    assert_eq!(body["instance_id"], serde_json::json!(service.lifecycle_instance().value()));
    Ok(())
}

#[test]
fn duplicate_operation_id_replays_instead_of_relaunching() -> Result<(), String> {
    let (mut service, state) = lifecycle_service()?;
    let body = launch_body(41, 7);
    let first = service.handle_request(&lifecycle_request("POST", &operations_path(), &body));
    let second = service.handle_request(&lifecycle_request("POST", &operations_path(), &body));
    assert_eq!(first.0, 200, "first launch must be accepted: {}", String::from_utf8_lossy(&first.1));
    assert_eq!(second.0, 200, "duplicate must replay, not fail");
    assert_eq!(first.1, second.1, "replay must be byte-identical");
    assert_eq!(
        recorded(&state)?.starts,
        1,
        "an exact duplicate must not launch a second process"
    );
    Ok(())
}

#[test]
fn stale_authority_epoch_is_rejected_before_any_process_effect() -> Result<(), String> {
    let (mut service, state) = lifecycle_service()?;
    let stale = launch_body(51, 7).replace("\"authority_epoch\":1", "\"authority_epoch\":9");
    let response = service.handle_request(&lifecycle_request("POST", &operations_path(), &stale));
    assert_eq!(response.0, 409);
    assert!(
        String::from_utf8_lossy(&response.1).contains("authority_epoch_stale"),
        "a stale epoch must be reported as stale"
    );
    assert_eq!(recorded(&state)?.starts, 0, "no process may be launched");
    Ok(())
}

#[test]
fn unknown_profile_id_is_rejected_before_any_process_effect() -> Result<(), String> {
    let (mut service, state) = lifecycle_service()?;
    let response = service.handle_request(&lifecycle_request(
        "POST",
        &operations_path(),
        &launch_body(61, 4242),
    ));
    assert_eq!(response.0, 422);
    assert!(String::from_utf8_lossy(&response.1).contains("profile_rejected"));
    assert_eq!(recorded(&state)?.starts, 0, "an unapproved profile must not launch");
    Ok(())
}

#[test]
fn unowned_attach_is_refused_without_a_process_effect() -> Result<(), String> {
    let (mut service, state) = lifecycle_service()?;
    let attach = r#"{"operation_id":71,"authority_epoch":1,"action":{"kind":"attach_existing","process":1,"pid":1001,"birth_id":5001,"executable":{"install_id":11,"executable_id":12,"image_id":13},"user_data":{"namespace_id":14}}}"#;
    let response = service.handle_request(&lifecycle_request("POST", &operations_path(), attach));
    assert_eq!(response.0, 403);
    assert!(
        String::from_utf8_lossy(&response.1).contains("unowned_attach"),
        "attaching to a process this gateway never authorized must be refused"
    );
    assert_eq!(recorded(&state)?.starts, 0);
    Ok(())
}

#[test]
fn malformed_and_conflicting_submissions_are_rejected_with_typed_codes() -> Result<(), String> {
    let (mut service, _state) = lifecycle_service()?;
    let cases = [
        (r#"{"operation_id":0,"authority_epoch":1,"action":{"kind":"launch_new","profile_id":7}}"#, 400),
        (r#"{"operation_id":1,"authority_epoch":0,"action":{"kind":"launch_new","profile_id":7}}"#, 400),
        (r#"{"operation_id":1,"authority_epoch":1,"action":{"kind":"unknown"}}"#, 400),
        (r#"{"operation_id":1,"authority_epoch":1,"action":{"kind":"stop"}}"#, 400),
        (r#"{"operation_id":1,"authority_epoch":1,"action":{"kind":"launch_new","profile_id":7},"instance_id":"other"}"#, 400),
    ];
    for (body, expected) in cases {
        let response = service.handle_request(&lifecycle_request("POST", &operations_path(), body));
        assert_eq!(response.0, expected, "body {body} must be rejected");
    }
    Ok(())
}

#[test]
fn operation_lookup_distinguishes_retained_from_absent() -> Result<(), String> {
    let (mut service, _state) = lifecycle_service()?;
    let launch = launch_body(81, 7);
    assert_eq!(
        service
            .handle_request(&lifecycle_request("POST", &operations_path(), &launch))
            .0,
        200
    );
    let found = service.handle_request(&lifecycle_request(
        "GET",
        &format!("{}/81", operations_path()),
        "",
    ));
    assert_eq!(found.0, 200);
    let body: serde_json::Value = serde_json::from_slice(&found.1).map_err(|error| error.to_string())?;
    assert_eq!(body["operation_id"], serde_json::json!(81));
    assert_eq!(body["sequence"], serde_json::json!(1));
    let missing = service.handle_request(&lifecycle_request(
        "GET",
        &format!("{}/82", operations_path()),
        "",
    ));
    assert_eq!(missing.0, 404);
    assert!(String::from_utf8_lossy(&missing.1).contains("operation_not_found"));
    Ok(())
}

#[test]
fn lifecycle_routes_require_the_configured_lease_fence() -> Result<(), String> {
    let (mut service, _state) = lifecycle_service()?;
    let mut request = lifecycle_request("POST", &operations_path(), &launch_body(91, 7));
    request
        .headers
        .insert(String::from("x-sts2-lease-epoch"), String::from("2"));
    let response = service.handle_request(&request);
    assert_eq!(response.0, 409, "a mismatched lease epoch must not mutate");
    assert!(String::from_utf8_lossy(&response.1).contains("lease_fence_rejected"));
    Ok(())
}

#[test]
fn lifecycle_routes_require_an_authorized_scope() -> Result<(), String> {
    let (mut service, _state) = lifecycle_service()?;
    let mut request = lifecycle_request("POST", &operations_path(), &launch_body(101, 7));
    request.headers.remove("authorization");
    assert_eq!(service.handle_request(&request).0, 401);
    let mut read = lifecycle_request("GET", &capability_path(), "");
    read.headers.remove("authorization");
    assert_eq!(service.handle_request(&read).0, 401);
    Ok(())
}

#[test]
fn stop_of_an_unowned_instance_reports_not_found() -> Result<(), String> {
    let (mut service, state) = lifecycle_service()?;
    let stop = r#"{"operation_id":111,"authority_epoch":1,"action":{"kind":"stop","mode":"graceful"}}"#;
    let response = service.handle_request(&lifecycle_request("POST", &operations_path(), stop));
    assert_eq!(response.0, 404);
    assert!(String::from_utf8_lossy(&response.1).contains("instance_not_found"));
    assert_eq!(recorded(&state)?.stops, 0);
    Ok(())
}

#[test]
fn liveness_is_decided_by_the_http_gate_not_the_lifecycle_fence() -> Result<(), String> {
    // The lifecycle fence port deliberately does not re-derive expiry (see ADR 0035): a deadline
    // compared inside it would be a weaker second copy of a decision `service_lease.rs` already
    // makes with more context. This test pins that split from both sides, so removing either half
    // fails here instead of silently leaving a lease that nothing enforces.
    let (mut service, state) = lifecycle_service()?;
    let body = launch_body(121, 7);

    // Gate closed: an inactive lease is refused before the lifecycle coordinator is reached.
    service.lease_active = false;
    let refused = service.handle_request(&lifecycle_request("POST", &operations_path(), &body));
    assert_eq!(refused.0, 409, "an inactive lease must be refused");
    assert!(String::from_utf8_lossy(&refused.1).contains("lease_not_active"));
    assert_eq!(
        recorded(&state)?.starts,
        0,
        "a refused lease must not reach the process port"
    );

    // Gate open: the same request is accepted, which proves the refusal above came from the gate
    // rather than from the fence port rejecting the bound lease's own identity.
    service.lease_active = true;
    let accepted = service.handle_request(&lifecycle_request("POST", &operations_path(), &body));
    assert_eq!(
        accepted.0,
        200,
        "an active lease must be admitted: {}",
        String::from_utf8_lossy(&accepted.1)
    );
    assert_eq!(recorded(&state)?.starts, 1);
    Ok(())
}
