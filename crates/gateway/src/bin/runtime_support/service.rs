// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_gateway::{
    GatewayRecoveryStore, RecoveryBootContext, RecoveryHostFence, RecoveryLease,
    RecoveryReleaseSet, RecoveryStoreConfig, RuntimeV2Binding, RuntimeV2CombatPhase,
    RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError, RuntimeV2Message,
    RuntimeV2Observation, RuntimeV2Status, RuntimeV2TransportFault,
};

use super::auth::{AuthFailure, AuthPolicy, AuthScope};
use super::coop_reports::CoopReports;
use super::forwarder::HttpRuntimeV2Forwarder;
use super::http::{HttpRequest, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, read_request, write_response};
use super::journal;
use super::metrics::RuntimeMetrics;
use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;
use super::runtime_v3_gameplay_forwarder::{
    RuntimeV3GameplayForwardError, RuntimeV3GameplayForwarder,
};

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:15525";
const DEFAULT_MOD_ADDRESS: &str = "127.0.0.1:15526";
const DEFAULT_OPERATION_CAPACITY: &str = "8";
const MAX_OPERATION_CAPACITY: usize = 64;
const DEFAULT_QUEUE_CAPACITY: &str = "8";
const MAX_QUEUE_CAPACITY: usize = 64;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct RuntimeService {
    config: RuntimeConfig,
    lease_active: bool,
    lease_revoked: bool,
    shutdown_requested: bool,
    runtime_v2: RuntimeV2Ledger<HttpRuntimeV2Forwarder>,
    runtime_v3: RuntimeV3GameplayForwarder,
    recovery_catalog: recovery_catalog::RecoveryCatalogCache,
    journal_path: Option<PathBuf>,
    _journal_lock: Option<journal::JournalLock>,
    metrics: RuntimeMetrics,
    coop_reports: Option<CoopReports>,
    recovery: Option<GatewayRecoveryStore>,
    recovery_boot: Option<RecoveryBootContext>,
    recovery_fence: Option<RecoveryHostFence>,
    recovery_lease: Option<RecoveryLease>,
    recovery_lease_deadline: Option<Instant>,
    recovery_lease_deadline_lease_id: Option<String>,
    recovery_host_grant: Option<HostLeaseGrant>,
    recovery_clock_started: Instant,
    recovery_clock_wall_millis: u64,
    recovery_last_now_millis: u64,
}

struct RuntimeConfig {
    listen_address: String,
    mod_address: String,
    auth_policy: AuthPolicy,
    mod_token: String,
    instance_id: String,
    caller_id: String,
    session_id: String,
    mcp_session_id: String,
    lease_id: String,
    lease_epoch: u64,
    operation_capacity: usize,
    queue_capacity: usize,
    journal_path: Option<PathBuf>,
    recovery_store_path: Option<PathBuf>,
    recovery_deployment_id: Option<String>,
    recovery_release: RecoveryReleaseSet,
    recovery_ttl_seconds: u64,
    recovery_renewal_interval_seconds: u64,
    host_lease_key: Vec<u8>,
    host_principal_id: String,
}

struct QueuedRequest {
    stream: TcpStream,
    request: HttpRequest,
}

#[path = "service_admission.rs"]
mod admission;
#[path = "service_authorization.rs"]
mod authorization;
#[path = "service_config.rs"]
mod configuration;
#[path = "service_coop.rs"]
mod coop;
#[path = "service_host_lease.rs"]
mod host_lease;
#[path = "service_host_lease_helpers.rs"]
mod host_lease_helpers;
#[path = "service_host_lease_ops.rs"]
mod host_lease_ops;
#[path = "service_lease.rs"]
mod lease;
#[path = "service_lease_transport.rs"]
mod lease_transport;
#[path = "service_recovery.rs"]
mod recovery;
#[path = "service_recovery_catalog.rs"]
mod recovery_catalog;
#[path = "service_recovery_dispatch.rs"]
mod recovery_dispatch;
#[path = "service_recovery_dispatch_host.rs"]
mod recovery_dispatch_host;
#[path = "service_recovery_dispatch_transport.rs"]
mod recovery_dispatch_transport;
#[path = "service_recovery_lease.rs"]
mod recovery_lease;
#[path = "service_recovery_ops.rs"]
mod recovery_ops;
#[path = "service_recovery_payload.rs"]
mod recovery_payload;
#[path = "service_recovery_receipt.rs"]
mod recovery_receipt;
#[path = "service_recovery_state.rs"]
mod recovery_state;
#[path = "service_recovery_v3.rs"]
mod recovery_v3;
#[path = "service_recovery_wire.rs"]
mod recovery_wire;
#[path = "service_routes.rs"]
mod routes;
#[path = "service_support.rs"]
mod support;
#[path = "service_v2.rs"]
mod v2;
#[path = "service_v3.rs"]
mod v3;

use admission::{accept_requests, run_worker};
use authorization::request_rejection;
use support::{
    json_bytes, json_error, json_overload, safe_identity, safe_operation_id, unix_millis,
};

#[derive(Clone, Debug)]
struct HostLeaseGrant {
    installation_id: String,
    grant_digest: String,
    grant: Value,
}

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
            shutdown_requested: false,
            runtime_v2,
            runtime_v3: RuntimeV3GameplayForwarder::new(MAX_BODY_BYTES, MAX_RESPONSE_BYTES),
            recovery_catalog: recovery_catalog::RecoveryCatalogCache::default(),
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

#[cfg(test)]
#[path = "service_admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "service_auth_tests.rs"]
mod auth_tests;
#[cfg(test)]
#[path = "service_tests.rs"]
mod legacy_tests;
#[cfg(test)]
#[path = "service_recovery_catalog_tests.rs"]
mod recovery_catalog_tests;
#[cfg(test)]
#[path = "service_routes_tests.rs"]
mod routes_tests;
#[cfg(test)]
#[path = "service_v3_catalog_tests.rs"]
mod runtime_v3_catalog_tests;
#[cfg(test)]
#[path = "service_support_tests.rs"]
mod test_support;

#[cfg(test)]
#[path = "service_coop_tests.rs"]
mod coop_tests;
