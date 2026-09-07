// SPDX-License-Identifier: MIT

use super::recovery_catalog::{RecoveryCatalogCache, RecoveryCatalogKey};

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

    // A delayed legal-actions response for the already-consumed generation
    // cannot repopulate the executable cache after a dispatch.
    assert!(!cache.capture(key(1), &response(&key(1))));
    assert!(cache.current().is_none());
    assert!(cache.observe(key(3)));
    assert!(cache.current().is_none());
}
