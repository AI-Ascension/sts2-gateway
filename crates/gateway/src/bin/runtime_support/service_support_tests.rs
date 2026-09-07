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

    Ok(RuntimeService {
        config,
        lease_active: true,
        lease_revoked: false,
        shutdown_requested: false,
        runtime_v2,
        runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
        recovery_catalog: recovery_catalog::RecoveryCatalogCache::default(),
        journal_path: None,
        _journal_lock: None,
        metrics: super::super::metrics::RuntimeMetrics::default(),
        coop_reports: None,
        recovery: None,
        recovery_boot: None,
        recovery_fence: None,
        recovery_lease: None,
        recovery_lease_deadline: None,
        recovery_host_grant: None,
        recovery_clock_started: Instant::now(),
        recovery_clock_wall_millis: 0,
        recovery_last_now_millis: 0,
    })
}

pub(super) fn authenticated_request(path: &str) -> HttpRequest {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("authorization"),
        String::from("Bearer gateway-token"),
    );
    headers.insert(
        String::from("x-sts2-instance-id"),
        String::from("instance-1"),
    );
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

#[cfg(test)]
mod recovery_v3_duplicate_tests {
    use serde_json::json;
    use sts2_gateway::{
        GatewayRecoveryStore, RUNTIME_V3_SCHEMA_DIGEST, RecoveryIntentResult, RecoveryLeaseRequest,
        RecoveryOperationIntent, RecoveryOperationState, RecoveryUncertaintyReason,
        canonical_json_digest,
    };
    use uuid::Uuid;

    use super::super::RuntimeV3GameplayRoute;
    use super::test_service;

    const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
    const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
    const STATE: &str = "00000000-0000-4000-8000-000000000003";
    const OPERATION: &str = "00000000-0000-4000-8000-000000000004";

    #[test]
    fn unknown_duplicate_replays_original_binding_before_missing_cache() -> Result<(), String> {
        let path = std::env::temp_dir().join(format!(
            "sts2-gateway-v3-duplicate-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        ));
        let mut service = test_service()?;
        service.config.instance_id = INSTANCE.to_owned();
        let mut store = GatewayRecoveryStore::open(&path).map_err(|error| error.to_string())?;
        let boot = store
            .start_boot(
                DEPLOYMENT,
                INSTANCE,
                service.config.recovery_release.clone(),
                1_000,
            )
            .map_err(|error| error.to_string())?;
        let fence = store
            .complete_host_fence(&boot, 1_001)
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
                now_millis: 1_002,
                ttl_seconds: 30,
                renewal_interval_seconds: 10,
            })
            .map_err(|error| error.to_string())?;
        let installation_id = Uuid::new_v4().to_string();
        let grant_digest = "a".repeat(64);
        store
            .prepare_host_lease_install(
                &lease.lease_id,
                &installation_id,
                &grant_digest,
                &fence.host_fence_id,
                fence.fence_generation,
                1_003,
            )
            .map_err(|error| error.to_string())?;
        store
            .complete_host_lease_install(
                &lease.lease_id,
                &installation_id,
                &grant_digest,
                1,
                &Uuid::new_v4().to_string(),
                1_004,
            )
            .map_err(|error| error.to_string())?;
        let action = br#"{"action":{"kind":"end_turn"},"action_id":"action-end-turn"}"#;
        let operation = RecoveryOperationIntent {
            operation_id: OPERATION.to_owned(),
            deployment_id: lease.deployment_id.clone(),
            instance_id: lease.instance_id.clone(),
            instance_incarnation: lease.instance_incarnation.clone(),
            boot_id: lease.boot_id.clone(),
            authority_generation: lease.authority_generation,
            lease_id: lease.lease_id.clone(),
            lease_epoch: lease.lease_epoch,
            schema_digest: RUNTIME_V3_SCHEMA_DIGEST.to_owned(),
            canonical_json: action.to_vec(),
            payload_digest: canonical_json_digest(action).map_err(|error| error.to_string())?,
            expected_state_id: STATE.to_owned(),
            expected_generation: 0,
            catalog_digest: "b".repeat(64),
            now_millis: 1_003,
        };
        let proof = lease.proof();
        match store
            .record_intent(&proof, operation)
            .map_err(|error| error.to_string())?
        {
            RecoveryIntentResult::Created(_) => {}
            RecoveryIntentResult::Duplicate(_) => return Err(String::from("unexpected duplicate")),
        }
        store
            .mark_dispatched(&proof, INSTANCE, OPERATION, 1_004)
            .map_err(|error| error.to_string())?;
        store
            .record_outcome(
                INSTANCE,
                OPERATION,
                RecoveryOperationState::Unknown,
                None,
                None,
                None,
                Some(RecoveryUncertaintyReason::Timeout),
                1_005,
            )
            .map_err(|error| error.to_string())?;
        service.recovery = Some(store);
        service.recovery_boot = Some(boot);
        service.recovery_fence = Some(fence);
        service.recovery_lease = Some(lease);

        let mut request = super::authenticated_request("/v3/instances/unused/action");
        request
            .headers
            .insert(String::from("x-sts2-correlation-id"), String::from("corr"));
        let envelope = json!({
            "operation_id": OPERATION,
            "state_id": STATE,
            "generation": 0,
            "action": {"action_id": "action-end-turn", "action": {"kind": "end_turn"}}
        });
        let (status, body) = service.recovery_v3_dispatch(
            &request,
            RuntimeV3GameplayRoute::DispatchAction,
            envelope,
        );
        assert_eq!(status, 503);
        assert!(
            String::from_utf8(body)
                .map_err(|error| error.to_string())?
                .contains("UNKNOWN")
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("gateway-recovery.lock"));
        Ok(())
    }
}
