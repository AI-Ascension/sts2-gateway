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
    RecoveryReleaseSet, RecoveryStoreConfig, RuntimeV2Authority, RuntimeV2Binding,
    RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError,
    RuntimeV2Message, RuntimeV2Observation, RuntimeV2RecoveryCapabilities,
    RuntimeV2RecoveryContract, RuntimeV2RecoveryError, RuntimeV2Status, RuntimeV2TransportFault,
    SeededRunBinding, SeededRunLedger, SeededRunLedgerConfig,
};

use super::auth::{AuthFailure, AuthPolicy, AuthScope};
use super::coop_reports::CoopReports;
use super::forwarder::HttpRuntimeV2Forwarder;
use super::http::{HttpRequest, MAX_BODY_BYTES, MAX_RESPONSE_BYTES, read_request, write_response};
use super::journal;
use super::metrics::RuntimeMetrics;
use super::runtime_map::RuntimeMapRoute;
use super::runtime_map_forwarder::RuntimeMapForwardError;
use super::runtime_map_forwarder::{MAX_MAP_RESPONSE_BYTES, RuntimeMapForwarder};
use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;
use super::runtime_v3_gameplay_forwarder::{
    RuntimeV3GameplayForwardError, RuntimeV3GameplayForwarder,
};
use super::runtime_v4_expert::RuntimeV4ExpertRoute;
use super::runtime_v4_expert_forwarder::RuntimeV4ExpertForwarder;
use super::runtime_v4_expert_rest_action::RuntimeV4ExpertRestActionRoute;
use super::runtime_v4_expert_rest_action_forwarder::RuntimeV4ExpertRestActionForwarder;
use super::seeded_run_forwarder::HttpSeededRunForwarder;

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
    allocation_cleanup_lease_id: Option<String>,
    shutdown_requested: bool,
    runtime_v2: RuntimeV2Ledger<HttpRuntimeV2Forwarder>,
    runtime_v3: RuntimeV3GameplayForwarder,
    recovery_catalog: recovery_catalog::RecoveryCatalogCache,
    runtime_v4_expert: RuntimeV4ExpertForwarder,
    runtime_v4_expert_rest_action: RuntimeV4ExpertRestActionForwarder,
    runtime_map: RuntimeMapForwarder,
    seeded_run: SeededRunLedger<HttpSeededRunForwarder>,
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
    workflow_authority: Option<RuntimeV2Authority>,
}

struct QueuedRequest {
    stream: TcpStream,
    request: HttpRequest,
}

#[path = "service_admission.rs"]
mod admission;
#[path = "service_allocation_cleanup.rs"]
mod allocation_cleanup;
#[path = "service_allocation_context.rs"]
mod allocation_context;
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
#[path = "service_map.rs"]
mod map;
#[path = "service_receipt_query.rs"]
mod receipt_query;
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
#[path = "service_runtime.rs"]
mod runtime;
#[path = "service_seeded_run.rs"]
mod seeded_run;
#[path = "service_support.rs"]
mod support;
#[path = "service_v2.rs"]
mod v2;
#[path = "service_v3.rs"]
mod v3;
#[path = "service_v4_expert.rs"]
mod v4_expert;
#[path = "service_v4_expert_rest_action.rs"]
mod v4_expert_rest_action;
#[path = "service_workflow_authority.rs"]
mod workflow_authority;

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

#[cfg(test)]
include!("service_test_modules.rs");
#[cfg(test)]
#[path = "service_receipt_query_tests.rs"]
mod receipt_query_tests;

#[cfg(test)]
#[path = "service_map_tests.rs"]
mod map_tests;

#[cfg(test)]
#[path = "service_v4_expert_rest_action_tests.rs"]
mod v4_expert_rest_action_tests;
