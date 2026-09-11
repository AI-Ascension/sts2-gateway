// SPDX-License-Identifier: MIT

use super::runtime_v3_catalog_tests::{
    capture_old_catalog, cleanup, dispatch_envelope, json_body, recovery_service, runtime_request,
};
use serde_json::json;

use sts2_gateway::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryOperationIntent, RecoveryOperationState,
    RecoveryUncertaintyReason, canonicalize_recovery_action, sha256_hex,
};

#[test]
fn malformed_settled_receipt_persists_unknown_and_replays_without_dispatch() -> Result<(), String>
{
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "malformed-receipt")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;

    let canonical_action = canonicalize_recovery_action(
        &serde_json::to_vec(&dispatch["action"]).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let proof = lease.proof();
    let operation_id = dispatch["operation_id"]
        .as_str()
        .ok_or_else(|| String::from("operation id missing"))?;
    let operation_id = operation_id.to_owned();
    let now_millis = service.recovery_now_millis();
    let operation = {
        let store = service
            .recovery
            .as_mut()
            .ok_or_else(|| String::from("recovery store missing"))?;
        let intent = RecoveryOperationIntent {
            operation_id: operation_id.to_owned(),
            deployment_id: proof.deployment_id.clone(),
            instance_id: proof.instance_id.clone(),
            instance_incarnation: proof.instance_incarnation.clone(),
            boot_id: proof.boot_id.clone(),
            authority_generation: proof.authority_generation,
            lease_id: proof.lease_id.clone(),
            lease_epoch: proof.lease_epoch,
            schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
            payload_digest: sha256_hex(&canonical_action),
            canonical_json: canonical_action,
            expected_state_id: dispatch["state_id"]
                .as_str()
                .ok_or_else(|| String::from("state id missing"))?
                .to_owned(),
            expected_generation: dispatch["generation"]
                .as_u64()
                .ok_or_else(|| String::from("generation missing"))?,
            catalog_digest: "a".repeat(64),
            now_millis,
        };
        match store
            .record_intent(&proof, intent)
            .map_err(|error| error.to_string())?
        {
            RecoveryIntentResult::Created(_) => {}
            RecoveryIntentResult::Duplicate(_) => {
                return Err(String::from("unexpected duplicate operation"));
            }
        }
        store
            .mark_dispatched(
                &proof,
                &proof.instance_id,
                &operation_id,
                now_millis.saturating_add(1),
            )
            .map_err(|error| error.to_string())?
    };

    // The host claimed SETTLED but omitted the operation-specific ticket and
    // effect witness. This receipt is not settlement evidence: preserve the
    // durable operation as UNKNOWN with an explicit receipt-missing reason.
    let malformed_receipt = json!({
        "payload": {
            "operation": service.recovery_operation_ref_value(&operation),
            "result": {"status": "SETTLED"}
        }
    });
    let (status, body) = service.apply_host_operation_response(
        &proof,
        &operation,
        "malformed-receipt",
        malformed_receipt,
    );
    assert_eq!(status, 503);
    let response = json_body(&body)?;
    assert_eq!(response["payload"]["result"]["status"], "UNKNOWN");
    assert_eq!(
        response["payload"]["operation"]["uncertainty_reason"],
        "receipt_missing"
    );

    let stored = service
        .recovery
        .as_ref()
        .ok_or_else(|| String::from("recovery store missing"))?
        .lookup_operation(
            &proof.instance_id,
            &operation_id,
            &operation.payload_digest,
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("stored operation missing"))?;
    assert_eq!(stored.state, RecoveryOperationState::Unknown);
    assert_eq!(
        stored.uncertainty_reason,
        Some(RecoveryUncertaintyReason::ReceiptMissing)
    );

    // The exact Runtime-v3 retry is replay-only after the durable UNKNOWN
    // transition. It must not attempt another host mutation or consult the
    // now-consumed legal-action catalog.
    let dispatch_request = runtime_request(&service, &lease, "action", dispatch.clone())?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    let replay = json_body(&body)?;
    assert_eq!(replay["payload"]["result"]["status"], "UNKNOWN");
    assert_eq!(replay["payload"]["operation"]["operation_id"], operation_id);
    cleanup(service, &path);
    Ok(())
}
