// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let config = RuntimeConfig::from_environment()?;
        let coop_reports = configuration::coop_reports_from_environment()?;
        if coop_reports.is_some() && config.lease_epoch > 9_007_199_254_740_991 {
            return Err("co-op lease epoch exceeds the wire bound".to_owned());
        }
        let journal_lock = config
            .journal_path
            .as_deref()
            .map(journal::JournalLock::acquire)
            .transpose()?;
        let recovery_clock_started = Instant::now();
        let recovery_clock_wall_millis = unix_millis();
        let (recovery, recovery_boot) = if let (Some(path), Some(deployment_id)) = (
            config.recovery_store_path.as_deref(),
            config.recovery_deployment_id.as_deref(),
        ) {
            let store_config = RecoveryStoreConfig {
                lease_ttl_seconds: config.recovery_ttl_seconds,
                lease_renewal_interval_seconds: config.recovery_renewal_interval_seconds,
                ..RecoveryStoreConfig::default()
            };
            let mut store = GatewayRecoveryStore::open_with_config(path, store_config)
                .map_err(|error| format!("recovery store open failed: {error}"))?;
            store
                .integrity_check()
                .map_err(|error| format!("recovery store integrity check failed: {error}"))?;
            let boot = store
                .start_boot(
                    deployment_id,
                    &config.instance_id,
                    config.recovery_release.clone(),
                    recovery_clock_wall_millis,
                )
                .map_err(|error| format!("recovery boot failed: {error}"))?;
            (Some(store), Some(boot))
        } else {
            (None, None)
        };
        let binding = RuntimeV2Binding::new(
            &config.instance_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
            RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
        )
        .map_err(|error| format!("Runtime-v2 binding is invalid: {error}"))?;
        let forwarder = HttpRuntimeV2Forwarder::new(
            &config.mod_address,
            &config.mod_token,
            &config.instance_id,
            &config.caller_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
        );
        let seeded_binding = SeededRunBinding::new(
            &config.instance_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
            0,
        )
        .map_err(|error| format!("seeded-run binding is invalid: {error}"))?;
        let seeded_forwarder = HttpSeededRunForwarder::new(
            &config.mod_address,
            &config.mod_token,
            &config.instance_id,
            &config.caller_id,
            &config.session_id,
            &config.lease_id,
            config.lease_epoch,
        );
        let mut seeded_run = SeededRunLedger::new(
            SeededRunLedgerConfig::new(config.operation_capacity),
            seeded_binding,
            seeded_forwarder,
        )
        .map_err(|error| format!("seeded-run ledger is invalid: {error}"))?;
        if let Some(path) = config.journal_path.as_deref()
            && let Some(state) = journal::seeded_load(path)?
        {
            seeded_run
                .restore_state(state)
                .map_err(|error| format!("seeded-run journal state is invalid: {error}"))?;
        }
        let mut runtime_v2 = RuntimeV2Ledger::new(
            RuntimeV2LedgerConfig::new(config.operation_capacity),
            binding,
            forwarder,
        )
        .map_err(|error| format!("Runtime-v2 ledger is invalid: {error}"))?;
        if let Some(path) = config.journal_path.as_deref()
            && let Some(state) = journal::load(path)?
        {
            runtime_v2
                .restore_state(state)
                .map_err(|error| format!("Runtime-v2 journal state is invalid: {error}"))?;
        }

        Ok(Self {
            journal_path: config.journal_path.clone(),
            _journal_lock: journal_lock,
            config,
            lease_active: false,
            lease_revoked: false,
            allocation_cleanup_lease_id: None,
            shutdown_requested: false,
            runtime_v2,
            runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
            recovery_catalog: recovery_catalog::RecoveryCatalogCache::default(),
            runtime_v4_expert: RuntimeV4ExpertForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
            runtime_v4_expert_rest_action: RuntimeV4ExpertRestActionForwarder::new(
                MAX_BODY_BYTES,
                MAX_RESPONSE_BYTES,
            ),
            runtime_map: RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES),
            seeded_run,
            metrics: RuntimeMetrics::default(),
            coop_reports,
            recovery,
            recovery_boot,
            recovery_fence: None,
            recovery_lease: None,
            recovery_lease_deadline: None,
            recovery_lease_deadline_lease_id: None,
            recovery_host_grant: None,
            recovery_clock_started,
            recovery_clock_wall_millis,
            recovery_last_now_millis: recovery_clock_wall_millis,
        })
    }

    pub(crate) fn run(self) -> Result<(), String> {
        let listener = TcpListener::bind(&self.config.listen_address)
            .map_err(|error| format!("gateway bind failed: {error}"))?;
        let listener_address = listener
            .local_addr()
            .map_err(|error| format!("gateway address lookup failed: {error}"))?;
        println!(
            "sts2-gateway runtime listening on {} for instance {}",
            self.config.listen_address, self.config.instance_id
        );
        let (sender, receiver) = sync_channel(self.config.queue_capacity);
        let admission_open = Arc::new(AtomicBool::new(true));
        let worker_open = Arc::clone(&admission_open);
        let auth_policy = self.config.auth_policy.clone();
        let recovery_enabled = self.recovery.is_some();
        let metrics = self.metrics.clone();
        let instance_id = self.config.instance_id.clone();
        let worker = thread::Builder::new()
            .name(String::from("sts2-gateway-runtime-worker"))
            .spawn(move || run_worker(self, receiver, worker_open, listener_address))
            .map_err(|error| format!("gateway worker spawn failed: {error}"))?;
        let result = accept_requests(
            listener,
            sender,
            admission_open,
            auth_policy,
            instance_id,
            recovery_enabled,
            metrics,
        );
        match worker.join() {
            Ok(worker_result) => result.and(worker_result),
            Err(_) => Err(String::from("gateway worker panicked")),
        }
    }
}
