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
    RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
    RuntimeV2LedgerError, RuntimeV2Message, RuntimeV2Observation, RuntimeV2Status,
    RuntimeV2TransportFault,
};

use super::auth::{AuthFailure, AuthPolicy, AuthScope};
use super::coop_reports::CoopReports;
use super::forwarder::HttpRuntimeV2Forwarder;
use super::http::{
    HttpRequest, HttpResponse, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, ReadError, read_request,
    read_response_with_limit, write_request, write_response,
};
use super::journal;
use super::metrics::RuntimeMetrics;
use super::runtime_map::RuntimeMapRoute;
use super::runtime_map_forwarder::RuntimeMapForwardError;
use super::runtime_map_forwarder::{MAX_MAP_RESPONSE_BYTES, RuntimeMapForwarder};
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
    runtime_map: RuntimeMapForwarder,
    journal_path: Option<PathBuf>,
    _journal_lock: Option<journal::JournalLock>,
    metrics: RuntimeMetrics,
    coop_reports: Option<CoopReports>,
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
#[path = "service_lease.rs"]
mod lease;
#[path = "service_map.rs"]
mod map;
#[path = "service_routes.rs"]
mod routes;
#[path = "service_v2.rs"]
mod v2;
#[path = "service_v3.rs"]
mod v3;

use admission::{accept_requests, run_worker};
use authorization::request_rejection;

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
            runtime_map: RuntimeMapForwarder::new(MAX_MAP_RESPONSE_BYTES),
            metrics: RuntimeMetrics::default(),
            coop_reports,
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
            metrics,
        );
        match worker.join() {
            Ok(worker_result) => result.and(worker_result),
            Err(_) => Err(String::from("gateway worker panicked")),
        }
    }
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn safe_operation_id(value: &str) -> bool {
    safe_identity(value) && !value.contains('/')
}

fn json_bytes(value: &Value) -> Vec<u8> {
    match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) => b"{\"error_code\":\"serialization_failed\"}".to_vec(),
    }
}

fn json_error(code: &str) -> Vec<u8> {
    json_bytes(&json!({ "error_code": code }))
}

fn json_overload(code: &str) -> Vec<u8> {
    json_bytes(&json!({
        "error_code": code,
        "retryable": true,
        "retry_after_ms": 1000
    }))
}

fn read_error_status(error: ReadError) -> u16 {
    match error {
        ReadError::Timeout => 504,
        ReadError::Malformed | ReadError::Oversized => 502,
        ReadError::Unavailable => 503,
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
#[path = "service_routes_tests.rs"]
mod routes_tests;
#[cfg(test)]
#[path = "service_support_tests.rs"]
mod test_support;

#[cfg(test)]
#[path = "service_coop_tests.rs"]
mod coop_tests;
