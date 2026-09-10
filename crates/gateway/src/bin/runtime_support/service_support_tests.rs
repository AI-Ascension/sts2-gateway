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
        shutdown_requested: false,
        runtime_v2,
        runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
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
