// SPDX-License-Identifier: MIT

use super::*;

fn fixture(name: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/runtime-v3-gameplay/golden")
        .join(name);
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn headers(value: &Value) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (field, header) in [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ] {
        headers.insert(
            header.to_owned(),
            value[field].as_str().unwrap_or_default().to_owned(),
        );
    }
    headers.insert(
        "x-sts2-lease-epoch".to_owned(),
        value["lease_epoch"].to_string(),
    );
    headers
}

#[test]
fn requests_require_route_kind_and_authenticated_envelope() -> Result<(), Box<dyn std::error::Error>>
{
    let forwarder = RuntimeV3GameplayForwarder::new(16 * 1024, 128 * 1024);
    let original = fixture("state-request.json")?;
    let headers = headers(&original);
    assert!(
        forwarder
            .validate_request(
                RuntimeV3GameplayRoute::State,
                &serde_json::to_vec(&original)?,
                &headers
            )
            .is_ok()
    );
    for field in [
        "instance_id",
        "session_id",
        "lease_id",
        "correlation_id",
        "kind",
        "schema_digest",
    ] {
        let mut value = original.clone();
        value[field] = Value::String("wrong".to_owned());
        assert!(
            forwarder
                .validate_request(
                    RuntimeV3GameplayRoute::State,
                    &serde_json::to_vec(&value)?,
                    &headers
                )
                .is_err(),
            "{field}"
        );
    }
    Ok(())
}

#[test]
fn requests_reject_stale_epoch_wrong_route_and_malformed_envelope()
-> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV3GameplayForwarder::new(16 * 1024, 128 * 1024);
    let original = fixture("state-request.json")?;
    let headers = headers(&original);
    let mut value = original.clone();
    value["lease_epoch"] = 2.into();
    assert!(
        forwarder
            .validate_request(
                RuntimeV3GameplayRoute::State,
                &serde_json::to_vec(&value)?,
                &headers
            )
            .is_err()
    );
    assert!(
        forwarder
            .validate_request(
                RuntimeV3GameplayRoute::DispatchAction,
                &serde_json::to_vec(&original)?,
                &headers
            )
            .is_err()
    );
    assert!(
        forwarder
            .validate_request(RuntimeV3GameplayRoute::State, b"{}", &headers)
            .is_err()
    );
    Ok(())
}

#[test]
fn strict_schema_rejects_missing_extra_duplicate_and_malformed_payloads()
-> Result<(), Box<dyn std::error::Error>> {
    let original = fixture("dispatch-action-request.json")?;
    assert!(validate_envelope(&serde_json::to_vec(&original)?).is_some());
    let mut value = original.clone();
    value["untrusted"] = true.into();
    assert!(validate_envelope(&serde_json::to_vec(&value)?).is_none());
    let mut value = original.clone();
    value.as_object_mut().ok_or("object")?.remove("observation");
    assert!(validate_envelope(&serde_json::to_vec(&value)?).is_none());
    let mut value = original.clone();
    value["action"]["action"]["arbitrary"] = true.into();
    assert!(validate_envelope(&serde_json::to_vec(&value)?).is_none());
    let encoded = serde_json::to_string(&original)?;
    let duplicate = encoded.replacen('{', "{\"operation_id\":\"other\",", 1);
    assert!(validate_envelope(duplicate.as_bytes()).is_none());
    let mut value = original;
    value["generation"] = 9_007_199_254_740_992_u64.into();
    assert!(validate_envelope(&serde_json::to_vec(&value)?).is_none());
    Ok(())
}

#[test]
fn responses_bind_correlation_context_kind_operation_and_witness()
-> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV3GameplayForwarder::new(16 * 1024, 128 * 1024);
    let request = fixture("dispatch-action-request.json")?;
    let original = fixture("dispatch-action-settled.json")?;
    let route = RuntimeV3GameplayRoute::DispatchAction;
    assert_eq!(
        forwarder.validate_response(route, &request, &serde_json::to_vec(&original)?),
        Ok(())
    );
    for field in [
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "operation_id",
        "kind",
    ] {
        let mut value = original.clone();
        value[field] = "wrong".into();
        assert!(
            forwarder
                .validate_response(route, &request, &serde_json::to_vec(&value)?)
                .is_err(),
            "{field}"
        );
    }
    let mut value = original.clone();
    value["observation"]["generation"] = 7.into();
    assert!(
        forwarder
            .validate_response(route, &request, &serde_json::to_vec(&value)?)
            .is_err()
    );
    let mut value = original.clone();
    value["transition"]["to_generation"] = 8.into();
    assert!(
        forwarder
            .validate_response(route, &request, &serde_json::to_vec(&value)?)
            .is_err()
    );
    let mut value = original;
    value["transition"] = Value::Null;
    assert!(
        forwarder
            .validate_response(route, &request, &serde_json::to_vec(&value)?)
            .is_err()
    );
    Ok(())
}

#[test]
fn semantic_bounds_reject_duplicate_catalog_ids_and_oversized_utf8()
-> Result<(), Box<dyn std::error::Error>> {
    let mut response = fixture("state-response.json")?;
    let action = fixture("dispatch-action-request.json")?["action"].clone();
    response["legal_actions"] = serde_json::json!([action, action]);
    assert!(validate_envelope(&serde_json::to_vec(&response)?).is_none());
    response["legal_actions"] = serde_json::json!([]);
    response["observation"]["visible_seed"] = "é".repeat(300).into();
    assert!(validate_envelope(&serde_json::to_vec(&response)?).is_none());
    Ok(())
}

#[test]
fn all_six_routes_match_only_exact_methods_instances_and_suffixes() {
    for (method, suffix, route) in [
        ("GET", "state", RuntimeV3GameplayRoute::State),
        ("GET", "legal-actions", RuntimeV3GameplayRoute::LegalActions),
        ("POST", "action", RuntimeV3GameplayRoute::DispatchAction),
        ("POST", "wait", RuntimeV3GameplayRoute::WaitForTransition),
        ("GET", "reobserve", RuntimeV3GameplayRoute::Reobserve),
        ("POST", "recover", RuntimeV3GameplayRoute::Recover),
    ] {
        let path = format!("/v3/instances/instance-1/{suffix}");
        assert_eq!(
            RuntimeV3GameplayRoute::parse(method, &path, "instance-1"),
            Some(route)
        );
        assert_eq!(
            RuntimeV3GameplayRoute::parse("DELETE", &path, "instance-1"),
            None
        );
        assert_eq!(
            RuntimeV3GameplayRoute::parse(method, &path, "instance-2"),
            None
        );
        assert_eq!(
            RuntimeV3GameplayRoute::parse(method, &format!("{path}/extra"), "instance-1"),
            None
        );
        assert_eq!(
            RuntimeV3GameplayRoute::parse(method, &format!("{path}?extra"), "instance-1"),
            None
        );
    }
}

#[test]
fn catalog_recovery_errors_are_explicit_narrow_and_correlated()
-> Result<(), Box<dyn std::error::Error>> {
    let forwarder = RuntimeV3GameplayForwarder::new(16 * 1024, 128 * 1024);
    let mut request = fixture("state-request.json")?;
    request["kind"] = "legal_actions_request".into();
    request["state_id"] = "combat-1".into();
    let route = RuntimeV3GameplayRoute::LegalActions;
    for (status, code) in [
        (409, "stale_generation"),
        (503, "host_not_configured"),
        (503, "host_observation_unavailable"),
    ] {
        let original = serde_json::json!({"correlation_id": request["correlation_id"],
            "error_code": code, "recovery": "reobserve"});
        let bytes = serde_json::to_vec(&original)?;
        assert!(forwarder.is_legal_actions_recovery(route, &request, status, &bytes));
        assert!(!forwarder.is_legal_actions_recovery(route, &request, 200, &bytes));
        assert!(!forwarder.is_legal_actions_recovery(
            route,
            &request,
            if status == 409 { 503 } else { 409 },
            &bytes
        ));
        assert!(!forwarder.is_legal_actions_recovery(
            RuntimeV3GameplayRoute::State,
            &request,
            status,
            &bytes
        ));
        for field in ["correlation_id", "error_code", "recovery"] {
            let mut value = original.clone();
            value[field] = "untrusted".into();
            assert!(!forwarder.is_legal_actions_recovery(
                route,
                &request,
                status,
                &serde_json::to_vec(&value)?
            ));
        }
        let mut value = original.clone();
        value["observation"] = serde_json::json!({});
        assert!(!forwarder.is_legal_actions_recovery(
            route,
            &request,
            status,
            &serde_json::to_vec(&value)?
        ));
        let duplicate =
            serde_json::to_string(&original)?.replacen('{', "{\"error_code\":\"secret\",", 1);
        assert!(!forwarder.is_legal_actions_recovery(
            route,
            &request,
            status,
            duplicate.as_bytes()
        ));
    }
    let oversized = vec![b' '; 1025];
    assert!(!forwarder.is_legal_actions_recovery(route, &request, 409, &oversized));
    Ok(())
}
