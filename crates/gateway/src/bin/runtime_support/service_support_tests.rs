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
    let runtime_v3 = RuntimeV3GameplayProxy::new(
        &config.mod_address,
        &config.mod_token,
        &config.instance_id,
        &config.caller_id,
        &config.session_id,
        &config.lease_id,
        config.lease_epoch,
        config.operation_capacity,
    );
    Ok(RuntimeService {
        config,
        lease_active: true,
        lease_revoked: false,
        shutdown_requested: false,
        runtime_v2,
        runtime_v3,
        journal_path: None,
        _journal_lock: None,
        metrics: super::super::metrics::RuntimeMetrics::default(),
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

pub(super) fn v3_action_body() -> Vec<u8> {
    br#"{"protocol_version":"runtime-v3-gameplay","schema_digest":"c961bbde893f0422f80233d14ea9ae8b648ee9032136e5370aa5f6b949f6575e","provenance":{"artifact":"sts2-protocol/runtime-v3-gameplay","source":"schemas/runtime-v3-gameplay.schema.json","generator":"hand-authored"},"correlation_id":"corr-state","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","lease_epoch":1,"generation":0,"kind":"action_request","operation_id":"op-card-1","observation":null,"action":{"action_id":"play_card","card_index":0,"target_id":null},"status":null,"error_code":null,"effect_witness":null}"#.to_vec()
}
