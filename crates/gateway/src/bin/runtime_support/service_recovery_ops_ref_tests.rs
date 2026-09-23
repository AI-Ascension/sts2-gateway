// SPDX-License-Identifier: MIT

// Regression coverage for the recovery operation-reference match on the
// historical-read and reconcile paths. `parse_operation_ref` returns the three
// parts `(operation_id, payload_digest, original_context)`, where the third
// element is only the context object. `operation_ref_matches`, however, expects
// the complete wire reference: it reads `operation_id`/`payload_digest` at the
// top level *and* `original_context.*`. Passing the context-only object makes
// the id/digest comparisons run against `null`, so every well-formed reference
// is refused with `recovery_operation_context_mismatch` (409) and the read path
// can never reach the host gate.

use super::super::recovery_frame::{RecoveryKind, request_frame};
use super::runtime_v3_catalog_tests::{cleanup, recovery_service_with_caller};
use super::test_support::authenticated_request;
use super::*;
use serde_json::{Value, json};
use sts2_gateway::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryLease, RecoveryOperation,
    RecoveryOperationIntent, canonicalize_recovery_action, sha256_hex,
};

const STATE: &str = "00000000-0000-4000-8000-000000000003";
const OPERATION: &str = "00000000-0000-4000-8000-000000000004";
// The recovery frame contract requires a UUID actor principal, so the fixture
// caller must not be the human-readable default.
const CALLER: &str = "00000000-0000-4000-8000-00000000000a";

fn record_operation(
    service: &mut RuntimeService,
    lease: &RecoveryLease,
    operation_id: &str,
) -> Result<RecoveryOperation, String> {
    let action = canonicalize_recovery_action(
        &serde_json::to_vec(&json!({
            "action": {"kind": "end_turn"},
            "action_id": "00000000-0000-4000-8000-000000000009",
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let intent = RecoveryOperationIntent {
        operation_id: operation_id.to_owned(),
        deployment_id: lease.deployment_id.clone(),
        instance_id: lease.instance_id.clone(),
        instance_incarnation: lease.instance_incarnation.clone(),
        boot_id: lease.boot_id.clone(),
        authority_generation: lease.authority_generation,
        lease_id: lease.lease_id.clone(),
        lease_epoch: lease.lease_epoch,
        schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
        payload_digest: sha256_hex(&action),
        canonical_json: action,
        expected_state_id: STATE.to_owned(),
        expected_generation: 1,
        catalog_digest: "0".repeat(64),
        now_millis: service.recovery_now_millis(),
    };
    let proof = lease.proof();
    let store = service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?;
    match store
        .record_intent(&proof, intent)
        .map_err(|error| error.to_string())?
    {
        RecoveryIntentResult::Created(operation) => Ok(operation),
        RecoveryIntentResult::Duplicate(_) => {
            Err(String::from("unexpected duplicate operation intent"))
        }
    }
}

fn reference_request(service: &RuntimeService, kind: RecoveryKind, payload: Value) -> HttpRequest {
    let mut request = authenticated_request("/v1/recovery/operation");
    request
        .headers
        .insert("content-type".into(), "application/json".into());
    request.headers.insert(
        "x-sts2-recovery-capability".into(),
        kind.capability().into(),
    );
    request.body = request_frame(
        kind,
        &service.config.caller_id,
        &uuid::Uuid::new_v4().to_string(),
        None,
        payload,
    );
    request
}

fn lookup_payload(reference: &Value) -> Value {
    json!({"operation": reference.clone(), "lookup_scope": "historical_read"})
}

fn reconcile_payload(service: &RuntimeService, reference: &Value) -> Result<Value, String> {
    let fence = service
        .recovery_fence
        .as_ref()
        .ok_or_else(|| String::from("recovery fence missing"))?;
    Ok(json!({
        "operation": reference.clone(),
        "strategy": "reobserve",
        "current_fence": super::recovery_wire::fence_value(fence),
    }))
}

/// A matching reference must clear the reference gate and reach the host read
/// authority check (which fails closed here because no read secret/host is
/// configured). Before the fix every reference died at the gate with 409.
fn assert_past_the_reference_gate(
    kind: RecoveryKind,
    status: u16,
    body: &[u8],
) -> Result<(), String> {
    let value: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    assert_eq!(
        value["error_code"],
        Value::Null,
        "a matching reference must not be refused at the reference gate: {value}"
    );
    assert_eq!(
        value["kind"],
        kind.response_name(),
        "expected a {kind:?} response frame, got {value}"
    );
    assert_eq!(value["payload"]["result"]["status"], "UNKNOWN");
    assert_eq!(status, 503);
    Ok(())
}

#[test]
fn lookup_accepts_a_well_formed_operation_reference() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service_with_caller(CALLER)?;
    let operation = record_operation(&mut service, &lease, OPERATION)?;
    let reference = service.recovery_operation_ref_value(&operation);
    let request = reference_request(
        &service,
        RecoveryKind::OperationLookup,
        lookup_payload(&reference),
    );
    let (status, body) = service.recovery_route(&request, RecoveryKind::OperationLookup);
    assert_past_the_reference_gate(RecoveryKind::OperationLookup, status, &body)?;
    cleanup(service, &path);
    Ok(())
}

#[test]
fn lookup_rejects_a_reference_with_a_mismatched_context() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service_with_caller(CALLER)?;
    let operation = record_operation(&mut service, &lease, OPERATION)?;
    let mut reference = service.recovery_operation_ref_value(&operation);
    // Same operation id/digest/instance, but a context that does not match the
    // durable operation: this is the case the gate must still refuse.
    reference["original_context"]["boot_id"] = uuid::Uuid::new_v4().to_string().into();
    let request = reference_request(
        &service,
        RecoveryKind::OperationLookup,
        lookup_payload(&reference),
    );
    let (status, body) = service.recovery_route(&request, RecoveryKind::OperationLookup);
    assert_eq!(status, 409);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "recovery_operation_context_mismatch");
    cleanup(service, &path);
    Ok(())
}

#[test]
fn reconcile_accepts_a_well_formed_operation_reference() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service_with_caller(CALLER)?;
    let operation = record_operation(&mut service, &lease, OPERATION)?;
    let reference = service.recovery_operation_ref_value(&operation);
    let payload = reconcile_payload(&service, &reference)?;
    let request = reference_request(&service, RecoveryKind::OperationReconcile, payload);
    let (status, body) = service.recovery_route(&request, RecoveryKind::OperationReconcile);
    assert_past_the_reference_gate(RecoveryKind::OperationReconcile, status, &body)?;
    cleanup(service, &path);
    Ok(())
}

#[test]
fn reconcile_rejects_a_reference_with_a_mismatched_context() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service_with_caller(CALLER)?;
    let operation = record_operation(&mut service, &lease, OPERATION)?;
    let mut reference = service.recovery_operation_ref_value(&operation);
    reference["original_context"]["lease_epoch"] = json!(operation.lease_epoch + 1);
    let payload = reconcile_payload(&service, &reference)?;
    let request = reference_request(&service, RecoveryKind::OperationReconcile, payload);
    let (status, body) = service.recovery_route(&request, RecoveryKind::OperationReconcile);
    assert_eq!(status, 409);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "recovery_operation_context_mismatch");
    cleanup(service, &path);
    Ok(())
}
