// SPDX-License-Identifier: MIT

use super::recovery_catalog::{RecoveryCatalogCache, RecoveryCatalogKey};
use super::runtime_v3_catalog_tests::{
    DISPATCH_OPERATION, OLD_STATE, capture_old_catalog, cleanup, dispatch_envelope, json_body,
    recovery_service, runtime_request,
};
use serde_json::{Value, json};
use sts2_gateway::{
    RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryOperationIntent,
    canonicalize_recovery_action, sha256_hex,
};

fn key(generation: u64) -> RecoveryCatalogKey {
    RecoveryCatalogKey {
        instance_id: String::from("instance"),
        instance_incarnation: String::from("incarnation"),
        session_id: String::from("session"),
        lease_id: String::from("lease"),
        lease_epoch: 4,
        state_id: String::from("state"),
        gameplay_generation: generation,
    }
}

fn response(key: &RecoveryCatalogKey) -> Vec<u8> {
    format!(
        r#"{{"instance_id":"{}","session_id":"{}","lease_id":"{}","lease_epoch":{},"kind":"legal_actions_response","state_id":"{}","generation":{},"legal_actions":[]}}"#,
        key.instance_id,
        key.session_id,
        key.lease_id,
        key.lease_epoch,
        key.state_id,
        key.gameplay_generation,
    )
    .into_bytes()
}

#[test]
fn catalog_context_must_match_before_a_replacement_is_cached() {
    let mut cache = RecoveryCatalogCache::default();
    let original = key(1);
    assert!(cache.capture(original.clone(), &response(&original)));
    let mut mismatched_key = original.clone();
    mismatched_key.lease_id = String::from("other-lease");
    assert!(!cache.capture(mismatched_key, &response(&original)));
    assert_eq!(cache.current().map(|(key, _, _)| key), Some(&original));
}

#[test]
fn newer_observation_invalidates_the_older_catalog() {
    let mut cache = RecoveryCatalogCache::default();
    let original = key(1);
    assert!(cache.capture(original, &response(&key(1))));
    let newer = key(2);
    assert!(cache.observe(newer));
    assert!(cache.current().is_none());
}

#[test]
fn generation_regression_is_rejected_without_eviction() {
    let mut cache = RecoveryCatalogCache::default();
    let newer = key(2);
    assert!(cache.capture(newer.clone(), &response(&newer)));
    assert!(!cache.observe(key(1)));
    assert_eq!(cache.current().map(|(key, _, _)| key), Some(&newer));
}

#[test]
fn same_generation_conflicting_state_is_rejected_without_eviction() {
    let mut cache = RecoveryCatalogCache::default();
    let original = key(2);
    assert!(cache.capture(original.clone(), &response(&original)));
    let mut conflicting = original.clone();
    conflicting.state_id = String::from("other-state");
    assert!(!cache.observe(conflicting));
    assert_eq!(cache.current().map(|(key, _, _)| key), Some(&original));
}

#[test]
fn same_generation_same_state_observation_keeps_the_catalog() {
    let mut cache = RecoveryCatalogCache::default();
    let original = key(2);
    assert!(cache.capture(original.clone(), &response(&original)));
    assert!(cache.observe(original.clone()));
    assert_eq!(cache.current().map(|(key, _, _)| key), Some(&original));
}

#[test]
fn invalidation_keeps_the_observation_watermark() {
    let mut cache = RecoveryCatalogCache::default();
    let original = key(2);
    assert!(cache.capture(original.clone(), &response(&original)));
    cache.invalidate_current();
    assert!(cache.current().is_none());

    // A delayed legal-actions response for the exact consumed observation is
    // not a fresh read and cannot restore executable actions.
    assert!(!cache.capture(original.clone(), &response(&original)));
    assert!(cache.current().is_none());

    // A delayed legal-actions response for the already-consumed generation
    // cannot repopulate the executable cache after a dispatch.
    assert!(!cache.capture(key(1), &response(&key(1))));
    assert!(cache.current().is_none());

    // A fresh authoritative observation of the same boundary explicitly
    // reopens that boundary for a new legal-actions read.
    assert!(cache.observe(original.clone()));
    assert!(cache.capture(original.clone(), &response(&original)));

    cache.invalidate_current();
    assert!(cache.observe(key(3)));
    assert!(cache.current().is_none());
    assert!(cache.capture(key(3), &response(&key(3))));
}

#[test]
fn accepted_replay_rejects_delayed_same_key_catalog_for_new_operation() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "accepted-correlation")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;

    // Seed the durable state reached after a host ACCEPTED response. The
    // catalog is consumed at dispatch time, before that response is observed.
    let action = canonicalize_recovery_action(
        &serde_json::to_vec(&dispatch["action"]).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let operation = RecoveryOperationIntent {
        operation_id: DISPATCH_OPERATION.to_owned(),
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
        expected_state_id: OLD_STATE.to_owned(),
        expected_generation: 1,
        catalog_digest: "0".repeat(64),
        now_millis: service.recovery_now_millis(),
    };
    let proof = lease.proof();
    let fence = service
        .recovery_fence
        .clone()
        .ok_or_else(|| String::from("recovery fence missing"))?;
    let dispatched_at = service.recovery_now_millis();
    let ticket_at = service.recovery_now_millis();
    let store = service
        .recovery
        .as_mut()
        .ok_or_else(|| String::from("recovery store missing"))?;
    match store
        .record_intent(&proof, operation)
        .map_err(|error| error.to_string())?
    {
        RecoveryIntentResult::Created(_) => {}
        RecoveryIntentResult::Duplicate(_) => return Err(String::from("unexpected duplicate")),
    }
    let dispatched = store
        .mark_dispatched(
            &proof,
            &lease.instance_id,
            DISPATCH_OPERATION,
            dispatched_at,
        )
        .map_err(|error| error.to_string())?;
    let ticket = store
        .issue_admission_ticket(
            &proof,
            &fence,
            &lease.instance_id,
            DISPATCH_OPERATION,
            &dispatched.payload_digest,
            ticket_at,
            lease.ttl_seconds.saturating_sub(1),
        )
        .map_err(|error| error.to_string())?;
    let _ = store;

    service.invalidate_recovery_catalog();
    let mut host_operation = service.recovery_operation_ref_value(&dispatched);
    host_operation["state"] = "ACCEPTED".into();
    host_operation["ticket"] = super::recovery_payload::ticket_value(&ticket);
    host_operation["witness"] = Value::Null;
    let accepted_response = json!({
        "payload": {
            "result": {"status": "ACCEPTED"},
            "operation": host_operation,
        }
    });
    let (status, _) = service.apply_host_operation_response(
        &proof,
        &dispatched,
        "accepted-correlation",
        accepted_response,
    );
    assert_eq!(status, 200);
    assert!(capture_old_catalog(&mut service, &lease, &dispatch).is_err());

    // The exact accepted operation remains replayable without consulting the
    // executable catalog, while a different operation at that old boundary
    // requires a fresh legal-actions read.
    let dispatch_request = runtime_request(&service, &lease, "action", dispatch.clone())?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    assert_eq!(json_body(&body)?["payload"]["result"]["status"], "ACCEPTED");

    let mut new_dispatch = dispatch;
    new_dispatch["operation_id"] = "00000000-0000-4000-8000-000000000008".into();
    new_dispatch["correlation_id"] = "new-accepted-correlation".into();
    let new_request = runtime_request(&service, &lease, "action", new_dispatch)?;
    let (status, body) = service.handle_request(&new_request);
    assert_eq!(status, 409);
    assert_eq!(
        json_body(&body)?["error_code"],
        "recovery_catalog_fresh_read_required"
    );
    cleanup(service, &path);
    Ok(())
}
