// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn run_runtime_v3_translation_case(
    state_ok: bool,
    witness_state_id: &str,
    witness_generation: u64,
    state_delay: Duration,
    expire_during_query: bool,
) -> Result<(), String> {
    run_runtime_v3_translation_case_with_host_status(
        state_ok,
        witness_state_id,
        witness_generation,
        state_delay,
        expire_during_query,
        "SETTLED",
    )
}

pub(super) fn run_runtime_v3_translation_case_with_host_status(
    state_ok: bool,
    witness_state_id: &str,
    witness_generation: u64,
    state_delay: Duration,
    expire_during_query: bool,
    host_status: &'static str,
) -> Result<(), String> {
    let (mut service, lease, path) = super::super::super::runtime_v3_catalog_tests::
        recovery_service_with_caller("00000000-0000-4000-8000-000000000008")?;
    let mut dispatch = super::super::super::runtime_v3_catalog_tests::dispatch_envelope(
        &service,
        &lease,
        "3",
    )?;
    super::super::super::runtime_v3_catalog_tests::capture_old_catalog(
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
    let witness_matches = witness_state_id == STATE && witness_generation == 2;
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
            super::super::super::super::recovery_frame::RecoveryKind::OperationIntent,
            super::super::super::super::recovery_frame::RecoveryKind::OperationDispatch,
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
            let forwarded = super::super::super::super::http::read_request(&mut stream)
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
                == super::super::super::super::recovery_frame::RecoveryKind::OperationIntent
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
                == super::super::super::super::recovery_frame::RecoveryKind::OperationIntent
            {
                super::super::super::super::recovery_frame::response_result(
                    "INTENT_RECORDED",
                    false,
                    None,
                )
            } else {
                super::super::super::super::recovery_frame::response_result(host_status, false, None)
            };
            let body = json!({
                "result": result,
                "operation": operation,
            });
            let response = super::super::super::super::recovery_frame::response_frame(
                kind,
                &correlation,
                "00000000-0000-4000-8000-000000000009",
                body,
            );
            super::super::super::super::http::write_response(&mut stream, 200, &response)
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
        let state_request = super::super::super::super::http::read_request(&mut stream)
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
            super::super::super::super::http::write_response(&mut stream, 503, b"{}")
                .map_err(|error| error.to_string())?;
            return Ok(correlations);
        }
        thread::sleep(state_delay);
        let mut state = super::super::super::runtime_v3_catalog_tests::fixture("state-response.json")?;
        for field in [
            "correlation_id",
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
        ] {
            state[field] = state_request[field].clone();
        }
        state["generation"] = witness_generation.into();
        state["state_id"] = STATE.into();
        state["observation"]["state_id"] = STATE.into();
        state["observation"]["generation"] = witness_generation.into();
        state["legal_actions"] = json!([]);
        super::super::super::super::http::write_response(
            &mut stream,
            200,
            &serde_json::to_vec(&state).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(correlations)
    });
    service.config.mod_address = address;
    if expire_during_query {
        service.recovery_lease_deadline = Some(Instant::now() + Duration::from_millis(100));
    }
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
    if host_status == "REJECTED" && state_ok && !expire_during_query {
        assert_eq!(status, 200, "body={}", String::from_utf8_lossy(&body));
        assert_eq!(response["kind"], "dispatch_action_response");
        assert_eq!(response["status"], "rejected");
        assert_eq!(response["operation_id"], operation_id);
        assert_eq!(response["error_code"], "recovery_operation_rejected");
        assert!(response["transition"].is_null());
        assert!(!response["state_id"].is_null());
        assert!(response["observation"].is_object());
        assert!(response["legal_actions"].is_array());
    } else if state_ok && witness_matches && !expire_during_query {
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
        let expected_error = translation_negative_tests::expected_error(state_ok, witness_matches);
        assert_eq!(response["error_code"], expected_error);
    }
    assert_eq!(correlations.len(), 2);
    assert_ne!(correlations[0], "3");
    assert_ne!(correlations[0], correlations[1]);
    super::super::super::runtime_v3_catalog_tests::cleanup(service, &path);
    Ok(())
}
