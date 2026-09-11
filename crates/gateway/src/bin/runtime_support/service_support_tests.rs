// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn test_service() -> Result<RuntimeService, String> {
    let config = RuntimeConfig {
        listen_address: String::from("127.0.0.1:15525"),
        mod_address: String::from("127.0.0.1:1"),
        auth_policy: AuthPolicy::test_all("gateway-token"),
        mod_token: String::from("mod-token"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        mcp_session_id: String::from("mcp-session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        operation_capacity: 8,
        queue_capacity: 8,
        journal_path: None,
        recovery_store_path: None,
        recovery_deployment_id: None,
        recovery_release: RecoveryReleaseSet::unconfigured(),
        recovery_ttl_seconds: 30,
        recovery_renewal_interval_seconds: 10,
        host_lease_key: vec![0x11; 32],
        host_principal_id: String::from("00000000-0000-4000-8000-00000000000a"),
        workflow_authority: None,
    };
    let binding = RuntimeV2Binding::new(
        &config.instance_id,
        &config.session_id,
        &config.lease_id,
        config.lease_epoch,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
    )
    .map_err(|error| error.to_string())?;
    let runtime_v2 = RuntimeV2Ledger::new(
        RuntimeV2LedgerConfig::new(config.operation_capacity),
        binding,
        HttpRuntimeV2Forwarder::new(
            &config.mod_address,
            &config.mod_token,
            &config.instance_id,
            &config.caller_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
        ),
    )
    .map_err(|error| error.to_string())?;
    let seeded_binding = SeededRunBinding::new(
        &config.instance_id,
        &config.session_id,
        &config.lease_id,
        config.lease_epoch,
        0,
    )
    .map_err(|error| error.to_string())?;
    let seeded_run = SeededRunLedger::new(
        SeededRunLedgerConfig::new(config.operation_capacity),
        seeded_binding,
        HttpSeededRunForwarder::new(
            &config.mod_address,
            &config.mod_token,
            &config.instance_id,
            &config.caller_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
        ),
    )
    .map_err(|error| error.to_string())?;

    Ok(RuntimeService {
        config,
        lease_active: true,
        lease_revoked: false,
        allocation_cleanup_lease_id: None,
        shutdown_requested: false,
        runtime_v2,
        runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        coop_native: CoopNativeForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        recovery_catalog: recovery_catalog::RecoveryCatalogCache::default(),
        runtime_v4_expert: RuntimeV4ExpertForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        runtime_v4_expert_rest_action: RuntimeV4ExpertRestActionForwarder::new(
            MAX_BODY_BYTES,
            MAX_RESPONSE_BYTES,
        ),
        runtime_map: RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES),
        seeded_run,
        journal_path: None,
        _journal_lock: None,
        metrics: super::super::metrics::RuntimeMetrics::default(),
        coop_reports: None,
        recovery: None,
        recovery_boot: None,
        recovery_fence: None,
        recovery_lease: None,
        recovery_lease_deadline: None,
        recovery_lease_deadline_lease_id: None,
        recovery_host_grant: None,
        recovery_clock_started: Instant::now(),
        recovery_clock_wall_millis: 0,
        recovery_last_now_millis: 0,
    })
}

pub(super) fn workflow_authority_service() -> Result<RuntimeService, String> {
    let mut service = test_service()?;
    let authority = RuntimeV2Authority::new("instance-1", "session-1", "lease-1", 1, "boot-1")
        .map_err(|error| error.to_string())?;
    let contract = RuntimeV2RecoveryContract::new(
        authority.clone(),
        RuntimeV2RecoveryCapabilities::unsupported(),
    )
    .map_err(|error| error.to_string())?;
    let forwarding = HttpRuntimeV2Forwarder::new(
        &service.config.mod_address,
        &service.config.mod_token,
        &service.config.instance_id,
        &service.config.caller_id,
        &service.config.session_id,
        &service.config.lease_id,
        service.config.lease_epoch,
    );
    service.runtime_v2 = RuntimeV2Ledger::new_with_recovery_contract(
        RuntimeV2LedgerConfig::new(service.config.operation_capacity),
        contract,
        service.runtime_v2.observation(),
        forwarding,
    )
    .map_err(|error| error.to_string())?;
    service.config.workflow_authority = Some(authority);
    Ok(service)
}

pub(super) fn authenticated_request(path: &str) -> HttpRequest {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("authorization"),
        String::from("Bearer gateway-token"),
    );
    headers.insert("x-sts2-instance-id".to_owned(), "instance-1".to_owned());
    headers.insert(String::from("x-sts2-caller-id"), String::from("harness"));
    headers.insert(String::from("x-sts2-session-id"), String::from("session-1"));
    headers.insert(
        String::from("x-mcp-session-id"),
        String::from("mcp-session-1"),
    );
    headers.insert(String::from("x-sts2-lease-id"), String::from("lease-1"));
    headers.insert(String::from("x-sts2-lease-epoch"), String::from("1"));
    headers.insert(
        String::from("x-sts2-correlation-id"),
        String::from("corr-state"),
    );
    HttpRequest {
        method: String::from("GET"),
        path: path.to_owned(),
        headers,
        body: Vec::new(),
    }
}

#[path = "service_recovery_v3_duplicate_tests.rs"]
mod recovery_v3_duplicate_tests;

#[cfg(test)]
mod recovery_catalog_tests {
    use serde_json::Value;
    use sts2_gateway::sha256_hex;

    use super::super::recovery_catalog::{
        RecoveryCatalogAdmission, RecoveryCatalogCache, RecoveryCatalogKey,
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

    fn response(key: &RecoveryCatalogKey, raw_actions: &str) -> Vec<u8> {
        format!(
            r#"{{"instance_id":"{}","session_id":"{}","lease_id":"{}","lease_epoch":{},"kind":"legal_actions_response","state_id":"{}","generation":{},"legal_actions":{raw_actions}}}"#,
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
    fn raw_catalog_bytes_are_hashed_without_value_reserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let raw = r#"[ {"action_id":"a\u0031","action":{"kind":"end_turn"}} ]"#;
        let mut cache = RecoveryCatalogCache::default();
        let catalog_key = key(9);
        assert!(cache.capture(catalog_key.clone(), &response(&catalog_key, raw)));
        let (_, bytes, digest) = cache.current().ok_or("catalog")?;
        assert_eq!(bytes, raw.as_bytes());
        assert_eq!(digest, sha256_hex(raw.as_bytes()));
        assert_ne!(
            digest,
            sha256_hex(br#"[{"action_id":"a1","action":{"kind":"end_turn"}}]"#)
        );
        Ok(())
    }

    #[test]
    fn raw_catalog_keeps_order_inner_whitespace_and_null_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let raw = r#"[
  { "action": { "target_id": null, "kind": "play_card" }, "action_id": "a\u0032" },
  { "action_id": "a3", "action": { "kind": "end_turn" } }
]"#;
        let mut cache = RecoveryCatalogCache::default();
        let catalog_key = key(10);
        assert!(cache.capture(catalog_key.clone(), &response(&catalog_key, raw)));
        let (_, bytes, digest) = cache.current().ok_or("catalog")?;
        assert_eq!(bytes, raw.as_bytes());
        assert_eq!(digest, sha256_hex(raw.as_bytes()));
        let action: Value = serde_json::from_str(
            r#"{"action_id":"a2","action":{"kind":"play_card","target_id":null}}"#,
        )?;
        assert_eq!(cache.admission(&key(10), &action), Ok(digest));
        Ok(())
    }

    #[test]
    fn malformed_duplicate_and_trailing_responses_do_not_poison_cache() {
        let mut cache = RecoveryCatalogCache::default();
        let catalog_key = key(1);
        assert!(cache.capture(catalog_key.clone(), &response(&catalog_key, "[]")));
        let before = cache
            .current()
            .map(|(_, bytes, digest)| (bytes.to_vec(), digest.to_owned()));
        assert!(!cache.capture(key(2), br#"{"kind":"legal_actions_response","state_id":"state","generation":2,"legal_actions":[],"legal_actions":[]}"#));
        assert!(
            !cache.capture(
                key(2),
                &response(&key(2), "[]")
                    .into_iter()
                    .chain(b" trailing".iter().copied())
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(
            cache
                .current()
                .map(|(_, bytes, digest)| (bytes.to_vec(), digest.to_owned())),
            before
        );
    }

    #[test]
    fn one_current_entry_requires_exact_key_and_action_membership()
    -> Result<(), Box<dyn std::error::Error>> {
        let action = r#"[{"action_id":"a1","action":{"kind":"end_turn"}}]"#;
        let mut cache = RecoveryCatalogCache::default();
        let catalog_key = key(1);
        assert!(cache.capture(catalog_key.clone(), &response(&catalog_key, action)));
        let legal: Value =
            serde_json::from_str(r#"{"action_id":"a1","action":{"kind":"end_turn"}}"#)?;
        let digest = sha256_hex(action.as_bytes());
        assert_eq!(cache.admission(&key(1), &legal), Ok(digest.as_str()));
        assert_eq!(
            cache.admission(&key(2), &legal),
            Err(RecoveryCatalogAdmission::Stale)
        );
        let changed: Value =
            serde_json::from_str(r#"{"action_id":"a1","action":{"kind":"proceed"}}"#)?;
        assert_eq!(
            cache.admission(&key(1), &changed),
            Err(RecoveryCatalogAdmission::ActionNotCurrent)
        );
        let newer_key = key(2);
        assert!(cache.capture(newer_key.clone(), &response(&newer_key, "[]")));
        assert_eq!(
            cache.admission(&key(1), &legal),
            Err(RecoveryCatalogAdmission::Stale)
        );
        Ok(())
    }

    #[test]
    fn empty_cache_requires_a_fresh_catalog() -> Result<(), Box<dyn std::error::Error>> {
        let action: Value =
            serde_json::from_str(r#"{"action_id":"a1","action":{"kind":"end_turn"}}"#)?;
        assert_eq!(
            RecoveryCatalogCache::default().admission(&key(1), &action),
            Err(RecoveryCatalogAdmission::Missing)
        );
        Ok(())
    }

    #[test]
    fn lease_and_incarnation_changes_make_the_cached_catalog_stale()
    -> Result<(), Box<dyn std::error::Error>> {
        let action = r#"[{"action_id":"a1","action":{"kind":"end_turn"}}]"#;
        let mut cache = RecoveryCatalogCache::default();
        let catalog_key = key(1);
        assert!(cache.capture(catalog_key.clone(), &response(&catalog_key, action)));
        let legal: Value =
            serde_json::from_str(r#"{"action_id":"a1","action":{"kind":"end_turn"}}"#)?;
        let mut changed_lease = key(1);
        changed_lease.lease_id = String::from("new-lease");
        assert_eq!(
            cache.admission(&changed_lease, &legal),
            Err(RecoveryCatalogAdmission::Stale)
        );
        let mut changed_incarnation = key(1);
        changed_incarnation.instance_incarnation = String::from("new-incarnation");
        assert_eq!(
            cache.admission(&changed_incarnation, &legal),
            Err(RecoveryCatalogAdmission::Stale)
        );
        Ok(())
    }
}
