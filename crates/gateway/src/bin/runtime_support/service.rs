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
use super::forwarder::HttpRuntimeV2Forwarder;
use super::http::{
    HttpRequest, HttpResponse, MAX_BODY_BYTES, ReadError, read_request, read_response,
    write_request, write_response,
};
use super::journal;
use super::metrics::RuntimeMetrics;
use super::runtime_v3_gameplay::RuntimeV3GameplayProxy;

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
    shutdown_requested: bool,
    runtime_v2: RuntimeV2Ledger<HttpRuntimeV2Forwarder>,
    runtime_v3: RuntimeV3GameplayProxy,
    journal_path: Option<PathBuf>,
    _journal_lock: Option<journal::JournalLock>,
    metrics: RuntimeMetrics,
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

impl RuntimeService {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let config = RuntimeConfig::from_environment()?;
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
        Ok(Self {
            journal_path: config.journal_path.clone(),
            _journal_lock: journal_lock,
            config,
            lease_active: false,
            shutdown_requested: false,
            runtime_v2,
            runtime_v3,
            metrics: RuntimeMetrics::default(),
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

    fn handle_queued_request(
        &mut self,
        mut stream: TcpStream,
        request: HttpRequest,
    ) -> Result<(), String> {
        let (status, body) = self.handle_request(&request);
        write_response(&mut stream, status, &body).map_err(|error| error.to_string())
    }

    fn cancel_queued_request(&self, mut stream: TcpStream) -> Result<(), String> {
        write_response(
            &mut stream,
            503,
            &json_error("runtime_v2_shutdown_admission_closed"),
        )
        .map_err(|error| error.to_string())
    }

    fn handle_request(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Some(rejection) =
            request_rejection(request, &self.config.auth_policy, &self.config.instance_id)
        {
            return rejection;
        }
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/health/ready") if request.body.is_empty() => self.health(),
            ("POST", "/v1/sessions/allocate")
                if request.content_type_is_json() && !request.body.is_empty() =>
            {
                self.allocate(&request.body)
            }
            ("GET", path) if path == self.state_path() && request.body.is_empty() => {
                self.relay_data(request, "GET", "/api/v1/runtime/state", &[])
            }
            ("POST", path) if path == self.action_path() && request.content_type_is_json() => {
                if request.body.is_empty() {
                    (400, json_error("action_body_required"))
                } else {
                    self.relay_data(request, "POST", "/api/v1/runtime/action", &request.body)
                }
            }
            ("POST", path)
                if path == self.runtime_v2_action_path() && request.content_type_is_json() =>
            {
                self.runtime_v2_action(request)
            }
            ("POST", path)
                if path == self.runtime_v3_action_path() && request.content_type_is_json() =>
            {
                self.runtime_v3_action(request)
            }
            ("GET", path) if path == self.runtime_v2_state_path() => self.runtime_v2_state(request),
            ("GET", path) if path == self.runtime_v3_state_path() && request.body.is_empty() => {
                self.runtime_v3_state(request)
            }
            ("GET", path) if path == self.runtime_v2_metrics_path() && request.body.is_empty() => {
                self.runtime_v2_metrics()
            }
            ("GET", path) if request.body.is_empty() => {
                if let Some(operation_id) = self.runtime_v3_operation_id(path) {
                    return self.runtime_v3_operation(request, operation_id);
                }
                let Some(operation_id) = self.runtime_v2_operation_id(path) else {
                    return (404, json_error("route_not_found"));
                };
                self.runtime_v2_reconcile(request, operation_id)
            }
            ("POST", path)
                if path == self.runtime_v2_shutdown_path() && request.body.is_empty() =>
            {
                self.runtime_v2_shutdown(request)
            }
            ("POST", path) if path == self.release_path() && request.body.is_empty() => {
                self.release(request)
            }
            _ => (404, json_error("route_not_found")),
        }
    }

    fn runtime_v2_metrics(&self) -> (u16, Vec<u8>) {
        (
            200,
            json_bytes(
                &self
                    .metrics
                    .snapshot(&self.config.instance_id, self.config.queue_capacity),
            ),
        )
    }

    fn runtime_v2_shutdown(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.lease_active = false;
        self.shutdown_requested = true;
        self.metrics.request_shutdown();
        (
            202,
            json_bytes(&json!({
                "status": "shutdown_requested",
                "instance_id": self.config.instance_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch
            })),
        )
    }

    fn state_path(&self) -> String {
        format!("/v1/instances/{}/state", self.config.instance_id)
    }

    fn action_path(&self) -> String {
        format!("/v1/instances/{}/action", self.config.instance_id)
    }

    fn release_path(&self) -> String {
        format!("/v1/instances/{}/release", self.config.instance_id)
    }

    fn runtime_v2_action_path(&self) -> String {
        format!("/v2/instances/{}/action", self.config.instance_id)
    }

    fn runtime_v2_state_path(&self) -> String {
        format!("/v2/instances/{}/state", self.config.instance_id)
    }

    fn runtime_v3_action_path(&self) -> String {
        format!("/v3/instances/{}/action", self.config.instance_id)
    }

    fn runtime_v3_state_path(&self) -> String {
        format!("/v3/instances/{}/state", self.config.instance_id)
    }

    fn runtime_v2_metrics_path(&self) -> String {
        format!("/v2/instances/{}/metrics", self.config.instance_id)
    }

    fn runtime_v2_shutdown_path(&self) -> String {
        format!("/v2/instances/{}/shutdown", self.config.instance_id)
    }

    fn runtime_v2_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("/v2/instances/{}/operations/", self.config.instance_id);
        path.strip_prefix(&prefix)
            .filter(|operation_id| !operation_id.is_empty() && !operation_id.contains('/'))
    }

    fn runtime_v3_operation_id<'a>(&self, path: &'a str) -> Option<&'a str> {
        let prefix = format!("/v3/instances/{}/operations/", self.config.instance_id);
        path.strip_prefix(&prefix)
            .filter(|operation_id| !operation_id.is_empty() && !operation_id.contains('/'))
    }

    fn health(&self) -> (u16, Vec<u8>) {
        match self.forward_mod("GET", "/health/ready", &[], None) {
            Ok(response) if response.status == 200 => (
                200,
                json_bytes(&json!({
                    "status": "ready",
                    "instance_id": self.config.instance_id,
                    "downstream": "ready"
                })),
            ),
            Ok(_) => (503, json_error("downstream_not_ready")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    fn allocate(&mut self, body: &[u8]) -> (u16, Vec<u8>) {
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return (400, json_error("allocation_body_invalid"));
        };
        let Some(object) = value.as_object() else {
            return (400, json_error("allocation_body_invalid"));
        };
        if object.len() != 3
            || object.get("instance_id").and_then(Value::as_str)
                != Some(self.config.instance_id.as_str())
            || object.get("caller_id").and_then(Value::as_str)
                != Some(self.config.caller_id.as_str())
            || object.get("session_id").and_then(Value::as_str)
                != Some(self.config.session_id.as_str())
        {
            return (409, json_error("allocation_identity_rejected"));
        }
        self.lease_active = true;
        (
            200,
            json_bytes(&json!({
                "status": "allocated",
                "instance_id": self.config.instance_id,
                "caller_id": self.config.caller_id,
                "session_id": self.config.session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "transport": "attached-loopback"
            })),
        )
    }

    fn release(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.lease_active = false;
        (
            200,
            json_bytes(&json!({
                "status": "released",
                "instance_id": self.config.instance_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch
            })),
        )
    }

    fn relay_data(
        &mut self,
        request: &HttpRequest,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if body.len() > MAX_BODY_BYTES
            || (!body.is_empty() && serde_json::from_slice::<Value>(body).is_err())
        {
            return (400, json_error("runtime_body_invalid"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        match self.forward_mod(method, path, body, correlation) {
            Ok(response) if response.body.len() <= MAX_BODY_BYTES => {
                (response.status, response.body)
            }
            Ok(_) => (502, json_error("downstream_response_oversized")),
            Err(status) => (status, json_error("downstream_unavailable")),
        }
    }

    fn runtime_v2_action(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if request.body.len() > MAX_BODY_BYTES {
            return (413, json_error("runtime_v2_body_oversized"));
        }
        let Ok(message) = serde_json::from_slice::<RuntimeV2Message>(&request.body) else {
            return (400, json_error("runtime_v2_request_invalid"));
        };
        if request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str)
            != Some(message.correlation_id.as_str())
        {
            return (409, json_error("runtime_v2_correlation_mismatch"));
        }
        let result = match self.journal_path.as_deref() {
            Some(path) => self
                .runtime_v2
                .submit_action_with_checkpoint(message, |state| {
                    journal::store(path, state).map_err(|_| ())
                }),
            None => self.runtime_v2.submit_action(message),
        };
        match result {
            Ok(response) => {
                if response.status == Some(RuntimeV2Status::Unknown) {
                    self.metrics.runtime_v2_unknown();
                }
                (runtime_v2_status(&response), runtime_v2_bytes(&response))
            }
            Err(error) => runtime_v2_error(error),
        }
    }

    fn runtime_v2_state(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let state_request = if request.body.is_empty() {
            RuntimeV2Message::state_request(
                self.runtime_v2.binding().metadata().clone(),
                correlation_id,
                &self.config.instance_id,
                &self.config.session_id,
                &self.config.lease_id,
                self.config.lease_epoch,
                self.runtime_v2.observation().generation,
            )
        } else {
            let Ok(message) = serde_json::from_slice::<RuntimeV2Message>(&request.body) else {
                return (400, json_error("runtime_v2_state_request_invalid"));
            };
            if message.kind != sts2_gateway::RuntimeV2MessageKind::StateRequest
                || message.validate().is_err()
                || message.correlation_id != correlation_id.as_str()
                || message.instance_id != self.config.instance_id
                || message.session_id != self.config.session_id
                || message.lease_id != self.config.lease_id
                || message.lease_epoch != self.config.lease_epoch
            {
                return (409, json_error("runtime_v2_state_identity_rejected"));
            }
            message
        };
        if state_request.validate().is_err() {
            return (500, json_error("runtime_v2_state_request_invalid"));
        }
        match self
            .runtime_v2
            .forwarding_mut()
            .forward_state(state_request.clone())
        {
            Ok(response) => match self
                .runtime_v2
                .accept_state_response(&state_request, response)
            {
                Ok(response) => {
                    if let Some(path) = self.journal_path.as_deref()
                        && journal::store(path, &self.runtime_v2.persisted_state()).is_err()
                    {
                        return (
                            503,
                            runtime_v2_state_unavailable(
                                &state_request,
                                "runtime_v2_persistence_failed",
                            ),
                        );
                    }
                    (200, runtime_v2_bytes(&response))
                }
                Err(_) => (
                    502,
                    runtime_v2_state_unavailable(
                        &state_request,
                        "downstream_state_response_invalid",
                    ),
                ),
            },
            Err(error) => (
                runtime_v2_state_status(error),
                runtime_v2_state_unavailable(&state_request, runtime_v2_state_reason(error)),
            ),
        }
    }

    fn runtime_v2_reconcile(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_identity(operation_id) {
            return (400, json_error("runtime_v2_operation_invalid"));
        }
        let Some(correlation_id) = request.headers.get("x-sts2-correlation-id") else {
            return (400, json_error("correlation_required"));
        };
        let message = RuntimeV2Message::reconcile_request(
            self.runtime_v2.binding().metadata().clone(),
            correlation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            self.runtime_v2.observation().generation,
            operation_id,
        );
        match self.runtime_v2.reconcile(message) {
            Ok(response) => {
                if response.status == Some(RuntimeV2Status::Unknown) {
                    self.metrics.runtime_v2_unknown();
                }
                if let Some(path) = self.journal_path.as_deref()
                    && journal::store(path, &self.runtime_v2.persisted_state()).is_err()
                {
                    return (503, json_error("runtime_v2_persistence_failed"));
                }
                (runtime_v2_status(&response), runtime_v2_bytes(&response))
            }
            Err(error) => runtime_v2_error(error),
        }
    }

    fn runtime_v3_action(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.runtime_v3.action(
            request,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    fn runtime_v3_state(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        self.runtime_v3.state(
            request,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    fn runtime_v3_operation(
        &mut self,
        request: &HttpRequest,
        operation_id: &str,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !safe_identity(operation_id) {
            return (400, json_error("runtime_v3_operation_invalid"));
        }
        self.runtime_v3.operation(
            request,
            operation_id,
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
        )
    }

    fn check_lease(&self, request: &HttpRequest) -> Result<(), (u16, Vec<u8>)> {
        if !self.lease_active {
            return Err((409, json_error("lease_not_active")));
        }
        let expected_epoch = self.config.lease_epoch.to_string();
        let expected = [
            ("x-sts2-instance-id", self.config.instance_id.as_str()),
            ("x-sts2-caller-id", self.config.caller_id.as_str()),
            ("x-sts2-session-id", self.config.session_id.as_str()),
            ("x-mcp-session-id", self.config.mcp_session_id.as_str()),
            ("x-sts2-lease-id", self.config.lease_id.as_str()),
            ("x-sts2-lease-epoch", expected_epoch.as_str()),
        ];
        if expected
            .iter()
            .any(|(name, value)| request.headers.get(*name).map(String::as_str) != Some(*value))
        {
            return Err((409, json_error("lease_fence_rejected")));
        }
        let Some(correlation) = request.headers.get("x-sts2-correlation-id") else {
            return Err((400, json_error("correlation_required")));
        };
        if !safe_identity(correlation) {
            return Err((400, json_error("correlation_invalid")));
        }
        Ok(())
    }

    fn forward_mod(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        correlation: Option<&str>,
    ) -> Result<HttpResponse, u16> {
        let expires = Instant::now() + Duration::from_secs(5);
        let address = self
            .config
            .mod_address
            .parse::<SocketAddr>()
            .map_err(|_| 503_u16)?;
        let mut stream =
            TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|_| 503_u16)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| 503_u16)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| 503_u16)?;
        let mut headers = BTreeMap::new();
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", self.config.mod_token),
        );
        headers.insert(String::from("Host"), self.config.mod_address.clone());
        headers.insert(String::from("Content-Length"), body.len().to_string());
        if !body.is_empty() {
            headers.insert(
                String::from("Content-Type"),
                String::from("application/json"),
            );
        }
        if let Some(correlation) = correlation {
            headers.insert(
                String::from("x-sts2-instance-id"),
                self.config.instance_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-caller-id"),
                self.config.caller_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-session-id"),
                self.config.session_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-lease-id"),
                self.config.lease_id.clone(),
            );
            headers.insert(
                String::from("x-sts2-lease-epoch"),
                self.config.lease_epoch.to_string(),
            );
            headers.insert(
                String::from("x-sts2-correlation-id"),
                correlation.to_owned(),
            );
        }
        write_request(&mut stream, method, path, &headers, body, expires).map_err(|_| 503_u16)?;
        read_response(&mut stream, expires).map_err(read_error_status)
    }
}

fn run_worker(
    mut service: RuntimeService,
    receiver: Receiver<QueuedRequest>,
    admission_open: Arc<AtomicBool>,
    listener_address: SocketAddr,
) -> Result<(), String> {
    while let Ok(queued) = receiver.recv() {
        let service_started = Instant::now();
        service.metrics.work_started();
        let result = service.handle_queued_request(queued.stream, queued.request);
        service.metrics.work_completed(service_started.elapsed());
        if let Err(error) = result {
            eprintln!("gateway queued request failed: {error}");
        }
        if service.shutdown_requested {
            admission_open.store(false, Ordering::Release);
            // Wake admission before draining. A listener may still be parsing a
            // request; retain the receiver until that producer has exited so a
            // concurrent successful enqueue always receives cancellation.
            wake_listener(listener_address);
            while let Ok(queued) = receiver.recv() {
                service.metrics.work_cancelled_on_shutdown();
                if let Err(error) = service.cancel_queued_request(queued.stream) {
                    eprintln!("gateway shutdown cancellation failed: {error}");
                }
            }
            return Ok(());
        }
    }
    admission_open.store(false, Ordering::Release);
    Ok(())
}

fn accept_requests(
    listener: TcpListener,
    sender: SyncSender<QueuedRequest>,
    admission_open: Arc<AtomicBool>,
    auth_policy: AuthPolicy,
    instance_id: String,
    metrics: RuntimeMetrics,
) -> Result<(), String> {
    loop {
        if !admission_open.load(Ordering::Acquire) {
            break;
        }
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("gateway accept failed: {error}"))?;
        if !admission_open.load(Ordering::Acquire) {
            break;
        }
        stream
            .set_read_timeout(Some(REQUEST_READ_TIMEOUT))
            .map_err(|error| format!("gateway request timeout setup failed: {error}"))?;
        stream
            .set_write_timeout(Some(REQUEST_WRITE_TIMEOUT))
            .map_err(|error| format!("gateway response timeout setup failed: {error}"))?;
        metrics.request_seen();
        let request = match read_request(&mut stream) {
            Ok(request) => request,
            Err(status) => {
                metrics.malformed_rejected();
                let _ = write_response(&mut stream, status, &json_error("malformed_request"));
                continue;
            }
        };
        if let Some((status, body)) = request_rejection(&request, &auth_policy, &instance_id) {
            if status == 401 || status == 403 {
                metrics.authentication_rejected();
            } else {
                metrics.malformed_rejected();
            }
            let _ = write_response(&mut stream, status, &body);
            continue;
        }
        let queued = QueuedRequest { stream, request };
        if !admission_open.load(Ordering::Acquire) {
            let mut stream = queued.stream;
            let _ = write_response(
                &mut stream,
                503,
                &json_error("runtime_v2_shutdown_admission_closed"),
            );
            break;
        }
        // Publish accounting before the receiver can consume the request.
        // Failed nonblocking sends roll back their reservation.
        metrics.queue_admitted();
        match sender.try_send(queued) {
            Ok(()) => {}
            Err(TrySendError::Full(queued)) => {
                metrics.queue_admission_reverted();
                metrics.queue_rejected();
                let mut stream = queued.stream;
                let _ = write_response(
                    &mut stream,
                    429,
                    &json_overload("runtime_v2_queue_capacity"),
                );
            }
            Err(TrySendError::Disconnected(queued)) => {
                metrics.queue_admission_reverted();
                let mut stream = queued.stream;
                let _ = write_response(
                    &mut stream,
                    503,
                    &json_error("runtime_v2_shutdown_admission_closed"),
                );
                break;
            }
        }
    }
    Ok(())
}

fn wake_listener(address: SocketAddr) {
    let _ = TcpStream::connect_timeout(&address, Duration::from_millis(200));
}

fn request_rejection(
    request: &HttpRequest,
    auth_policy: &AuthPolicy,
    instance_id: &str,
) -> Option<(u16, Vec<u8>)> {
    if !headers_are_allowed(&request.headers) {
        return Some((400, json_error("unsupported_header")));
    }
    match auth_policy.authorize(
        request.headers.get("authorization").map(String::as_str),
        required_scope(request, instance_id),
    ) {
        Ok(()) => None,
        Err(AuthFailure::Missing | AuthFailure::Invalid) => Some((401, json_error("unauthorized"))),
        Err(AuthFailure::Expired) => Some((401, json_error("token_expired"))),
        Err(AuthFailure::Scope) => Some((403, json_error("insufficient_scope"))),
    }
}

fn required_scope(request: &HttpRequest, instance_id: &str) -> AuthScope {
    let action_path = format!("/v2/instances/{instance_id}/action");
    let gameplay_action_path = format!("/v3/instances/{instance_id}/action");
    let legacy_action_path = format!("/v1/instances/{instance_id}/action");
    if request.method == "POST"
        && (request.path == action_path
            || request.path == gameplay_action_path
            || request.path == legacy_action_path)
    {
        return AuthScope::Mutate;
    }
    let allocate_path = "/v1/sessions/allocate";
    let release_path = format!("/v1/instances/{instance_id}/release");
    let shutdown_path = format!("/v2/instances/{instance_id}/shutdown");
    if request.method == "POST"
        && (request.path == allocate_path
            || request.path == release_path
            || request.path == shutdown_path)
    {
        return AuthScope::Control;
    }
    AuthScope::Read
}

impl RuntimeConfig {
    fn from_environment() -> Result<Self, String> {
        let listen_address = env_or_default("STS2_GATEWAY_ADDR", DEFAULT_LISTEN_ADDRESS)?;
        let mod_address = env_or_default("STS2_MOD_ADDR", DEFAULT_MOD_ADDRESS)?;
        validate_loopback_address("STS2_GATEWAY_ADDR", &listen_address)?;
        validate_loopback_address("STS2_MOD_ADDR", &mod_address)?;
        let auth_policy = AuthPolicy::from_environment()?;
        let mod_token = required("STS2_MOD_TOKEN")?;
        let instance_id = env_or_default("STS2_INSTANCE_ID", "instance-1")?;
        let caller_id = env_or_default("STS2_CALLER_ID", "harness")?;
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let mcp_session_id = env_or_default("STS2_MCP_SESSION_ID", &session_id)?;
        let lease_id = env_or_default("STS2_LEASE_ID", "lease-1")?;
        let lease_epoch = env_or_default("STS2_LEASE_EPOCH", "1")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?;
        let operation_capacity = parse_operation_capacity(&env_or_default(
            "STS2_RUNTIME_V2_OPERATION_CAPACITY",
            DEFAULT_OPERATION_CAPACITY,
        )?)?;
        let queue_capacity = parse_queue_capacity(&env_or_default(
            "STS2_RUNTIME_V2_QUEUE_CAPACITY",
            DEFAULT_QUEUE_CAPACITY,
        )?)?;
        let journal_path = optional_path("STS2_RUNTIME_V2_JOURNAL")?;
        for (name, value) in [
            ("STS2_INSTANCE_ID", &instance_id),
            ("STS2_CALLER_ID", &caller_id),
            ("STS2_SESSION_ID", &session_id),
            ("STS2_MCP_SESSION_ID", &mcp_session_id),
            ("STS2_LEASE_ID", &lease_id),
        ] {
            if !safe_identity(value) {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        for (name, value) in [("STS2_MOD_TOKEN", &mod_token)] {
            if value.is_empty()
                || value.len() > 256
                || value.bytes().any(|byte| byte.is_ascii_whitespace())
            {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        Ok(Self {
            listen_address,
            mod_address,
            auth_policy,
            mod_token,
            instance_id,
            caller_id,
            session_id,
            mcp_session_id,
            lease_id,
            lease_epoch,
            operation_capacity,
            queue_capacity,
            journal_path,
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn validate_loopback_address(name: &str, value: &str) -> Result<(), String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| format!("{name} must be a numeric loopback IP:port endpoint"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "{name} must be a numeric loopback IP:port endpoint"
        ));
    }
    Ok(())
}

fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn optional_path(name: &str) -> Result<Option<PathBuf>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(PathBuf::from(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn parse_operation_capacity(value: &str) -> Result<usize, String> {
    let capacity = value
        .parse::<usize>()
        .map_err(|_| String::from("STS2_RUNTIME_V2_OPERATION_CAPACITY must be an integer"))?;
    if capacity == 0 || capacity > MAX_OPERATION_CAPACITY {
        return Err(format!(
            "STS2_RUNTIME_V2_OPERATION_CAPACITY must be between 1 and {MAX_OPERATION_CAPACITY}"
        ));
    }
    Ok(capacity)
}

fn parse_queue_capacity(value: &str) -> Result<usize, String> {
    let capacity = value
        .parse::<usize>()
        .map_err(|_| String::from("STS2_RUNTIME_V2_QUEUE_CAPACITY must be an integer"))?;
    if capacity == 0 || capacity > MAX_QUEUE_CAPACITY {
        return Err(format!(
            "STS2_RUNTIME_V2_QUEUE_CAPACITY must be between 1 and {MAX_QUEUE_CAPACITY}"
        ));
    }
    Ok(capacity)
}

fn headers_are_allowed(headers: &BTreeMap<String, String>) -> bool {
    headers.keys().all(|name| {
        matches!(
            name.as_str(),
            "authorization"
                | "connection"
                | "content-length"
                | "content-type"
                | "host"
                | "x-mcp-request-id"
                | "x-mcp-session-id"
                | "x-sts2-instance-id"
                | "x-sts2-caller-id"
                | "x-sts2-session-id"
                | "x-sts2-lease-id"
                | "x-sts2-lease-epoch"
                | "x-sts2-correlation-id"
        )
    })
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
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

fn runtime_v2_bytes(message: &RuntimeV2Message) -> Vec<u8> {
    match serde_json::to_vec(message) {
        Ok(bytes) => bytes,
        Err(_) => json_error("runtime_v2_serialization_failed"),
    }
}

fn runtime_v2_status(message: &RuntimeV2Message) -> u16 {
    match message.status {
        Some(RuntimeV2Status::Rejected)
            if message.error_code.as_deref() == Some("idempotency_conflict") =>
        {
            409
        }
        Some(RuntimeV2Status::Rejected | RuntimeV2Status::Cancelled | RuntimeV2Status::Unknown) => {
            200
        }
        Some(RuntimeV2Status::Accepted | RuntimeV2Status::Settled) => 200,
        None => 200,
    }
}

fn runtime_v2_state_unavailable(request: &RuntimeV2Message, reason: &str) -> Vec<u8> {
    json_bytes(&json!({
        "status": "unavailable",
        "error_code": "sts2.runtime/state_unavailable",
        "reason": reason,
        "request": request,
    }))
}

fn runtime_v2_state_status(error: RuntimeV2TransportFault) -> u16 {
    match error {
        RuntimeV2TransportFault::TimeoutBeforeWrite
        | RuntimeV2TransportFault::TimeoutAfterWrite => 504,
        RuntimeV2TransportFault::MalformedResponse => 502,
        RuntimeV2TransportFault::UnavailableBeforeWrite
        | RuntimeV2TransportFault::RejectedBeforeWrite
        | RuntimeV2TransportFault::DisconnectedBeforeWrite
        | RuntimeV2TransportFault::DisconnectedAfterWrite
        | RuntimeV2TransportFault::ReceiptUnavailable => 503,
    }
}

fn runtime_v2_state_reason(error: RuntimeV2TransportFault) -> &'static str {
    match error {
        RuntimeV2TransportFault::UnavailableBeforeWrite => "downstream_unavailable_before_write",
        RuntimeV2TransportFault::RejectedBeforeWrite => "downstream_rejected_before_write",
        RuntimeV2TransportFault::TimeoutBeforeWrite => "downstream_timeout_before_write",
        RuntimeV2TransportFault::DisconnectedBeforeWrite => "downstream_disconnected_before_write",
        RuntimeV2TransportFault::TimeoutAfterWrite => "downstream_timeout_after_write",
        RuntimeV2TransportFault::DisconnectedAfterWrite => "downstream_disconnected_after_write",
        RuntimeV2TransportFault::MalformedResponse => "downstream_malformed_response",
        RuntimeV2TransportFault::ReceiptUnavailable => "downstream_receipt_unavailable",
    }
}

fn runtime_v2_error(error: RuntimeV2LedgerError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        RuntimeV2LedgerError::InvalidRequest(_) | RuntimeV2LedgerError::RequestDigest(_) => {
            (400, "runtime_v2_request_invalid")
        }
        RuntimeV2LedgerError::MissingOperationId => (400, "runtime_v2_operation_required"),
        RuntimeV2LedgerError::CapacityExceeded => {
            return (429, json_overload("runtime_v2_operation_capacity"));
        }
        RuntimeV2LedgerError::OperationNotFound => (404, "runtime_v2_operation_not_found"),
        RuntimeV2LedgerError::OperationInProgress => (409, "runtime_v2_operation_in_progress"),
        RuntimeV2LedgerError::Fence(_) => (409, "runtime_v2_lease_fence_rejected"),
        RuntimeV2LedgerError::StaleGeneration { .. } => (409, "runtime_v2_stale_generation"),
        RuntimeV2LedgerError::ZeroCapacity => (500, "runtime_v2_operation_capacity_invalid"),
        RuntimeV2LedgerError::PersistedStateMismatch
        | RuntimeV2LedgerError::PersistedStateInvalid => (500, "runtime_v2_journal_invalid"),
        RuntimeV2LedgerError::PersistenceFailed => (503, "runtime_v2_persistence_failed"),
    };
    (status, json_error(code))
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
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;
    use sts2_gateway::{
        RuntimeV2Binding, RuntimeV2CombatPhase, RuntimeV2Ledger, RuntimeV2LedgerConfig,
        RuntimeV2Message, RuntimeV2Metadata, RuntimeV2Observation,
    };

    use super::super::runtime_v3_gameplay::RuntimeV3GameplayProxy;
    use super::{
        AuthPolicy, AuthScope, HttpRequest, HttpRuntimeV2Forwarder, RuntimeConfig, RuntimeService,
    };

    fn test_service() -> Result<RuntimeService, String> {
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
            shutdown_requested: false,
            runtime_v2,
            runtime_v3,
            journal_path: None,
            _journal_lock: None,
            metrics: super::super::metrics::RuntimeMetrics::default(),
        })
    }

    fn authenticated_request(path: &str) -> HttpRequest {
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

    fn v3_action_body() -> Vec<u8> {
        br#"{"protocol_version":"runtime-v3-gameplay","schema_digest":"c961bbde893f0422f80233d14ea9ae8b648ee9032136e5370aa5f6b949f6575e","provenance":{"artifact":"sts2-protocol/runtime-v3-gameplay","source":"schemas/runtime-v3-gameplay.schema.json","generator":"hand-authored"},"correlation_id":"corr-state","instance_id":"instance-1","session_id":"session-1","lease_id":"lease-1","lease_epoch":1,"generation":0,"kind":"action_request","operation_id":"op-card-1","observation":null,"action":{"action_id":"play_card","card_index":0,"target_id":null},"status":null,"error_code":null,"effect_witness":null}"#.to_vec()
    }

    #[test]
    fn state_route_returns_typed_request_and_explicit_unavailable_fallback() -> Result<(), String> {
        let mut service = test_service()?;
        let request = authenticated_request("/v2/instances/instance-1/state");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["status"], "unavailable");
        assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
        assert_eq!(value["reason"], "downstream_unavailable_before_write");
        assert_eq!(value["request"]["kind"], "state_request");
        assert_eq!(value["request"]["instance_id"], "instance-1");
        assert_eq!(value["request"]["correlation_id"], "corr-state");
        Ok(())
    }

    #[test]
    fn v2_gets_are_not_arbitrary_proxy_routes() -> Result<(), String> {
        let mut service = test_service()?;
        for path in [
            "/v2/instances/instance-1/state/extra",
            "/v2/instances/instance-1/not-a-proxy",
        ] {
            let (status, _) = service.handle_request(&authenticated_request(path));
            assert_eq!(status, 404, "unexpected route match for {path}");
        }
        Ok(())
    }

    #[test]
    fn v3_state_route_is_authenticated_and_fixed_to_the_gameplay_profile() -> Result<(), String> {
        let mut service = test_service()?;
        let request = authenticated_request("/v3/instances/instance-1/state");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "runtime_v3_downstream_unavailable");

        let (status, _) = service.handle_request(&authenticated_request(
            "/v3/instances/instance-1/state/extra",
        ));
        assert_eq!(status, 404);
        Ok(())
    }

    #[test]
    fn v3_action_validates_the_new_profile_before_forwarding() -> Result<(), String> {
        let mut service = test_service()?;
        let mut request = authenticated_request("/v3/instances/instance-1/action");
        request.method = String::from("POST");
        request.headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        request.body = v3_action_body();
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "runtime_v3_downstream_unavailable");

        request.body = b"{}".to_vec();
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 400);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(
            value["error_code"],
            "runtime_v3_gameplay_unknown_or_missing_field"
        );
        Ok(())
    }

    #[test]
    fn v3_action_requires_mutate_scope() -> Result<(), String> {
        let mut service = test_service()?;
        service.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
        let mut request = authenticated_request("/v3/instances/instance-1/action");
        request.method = String::from("POST");
        request.headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        request.body = v3_action_body();
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 403);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "insufficient_scope");
        Ok(())
    }

    #[test]
    fn bearer_authentication_requires_an_exact_value() {
        let policy = AuthPolicy::test_all("gateway-token");
        assert!(
            policy
                .authorize(Some("Bearer gateway-token"), AuthScope::Read)
                .is_ok()
        );
        for value in [
            None,
            Some(""),
            Some("gateway-token"),
            Some("Bearer wrong-token"),
            Some("bearer gateway-token"),
            Some("Bearer gateway-token "),
            Some("Bearer gateway-token\n"),
        ] {
            assert!(policy.authorize(value, AuthScope::Read).is_err());
        }
    }

    #[test]
    fn operation_capacity_is_explicitly_bounded() {
        assert_eq!(super::parse_operation_capacity("1"), Ok(1));
        assert_eq!(
            super::parse_operation_capacity("64"),
            Ok(super::MAX_OPERATION_CAPACITY)
        );
        assert!(super::parse_operation_capacity("0").is_err());
        assert!(super::parse_operation_capacity("65").is_err());
        assert!(super::parse_operation_capacity("not-a-number").is_err());
    }

    #[test]
    fn queue_capacity_is_explicitly_bounded() {
        assert_eq!(super::parse_queue_capacity("1"), Ok(1));
        assert_eq!(
            super::parse_queue_capacity("64"),
            Ok(super::MAX_QUEUE_CAPACITY)
        );
        assert!(super::parse_queue_capacity("0").is_err());
        assert!(super::parse_queue_capacity("65").is_err());
        assert!(super::parse_queue_capacity("not-a-number").is_err());
    }

    #[test]
    fn runtime_endpoints_are_numeric_loopback_addresses() {
        for address in ["127.0.0.1:15525", "127.0.0.2:15526", "[::1]:15525"] {
            assert!(super::validate_loopback_address("endpoint", address).is_ok());
        }
        for address in [
            "0.0.0.0:15525",
            "[::]:15525",
            "192.0.2.1:15525",
            "localhost:15525",
            "example.com:80",
            "127.0.0.1",
            "127.0.0.1:99999",
        ] {
            assert!(super::validate_loopback_address("endpoint", address).is_err());
        }
    }

    #[test]
    fn operation_overload_is_typed_and_retryable() -> Result<(), String> {
        let (status, body) =
            super::runtime_v2_error(sts2_gateway::RuntimeV2LedgerError::CapacityExceeded);
        assert_eq!(status, 429);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "runtime_v2_operation_capacity");
        assert_eq!(value["retryable"], true);
        assert_eq!(value["retry_after_ms"], 1000);
        Ok(())
    }

    #[test]
    fn missing_and_wrong_authentication_fail_before_lease_processing() -> Result<(), String> {
        let mut service = test_service()?;
        let mut missing = authenticated_request("/health/ready");
        missing.headers.remove("authorization");
        assert_eq!(service.handle_request(&missing).0, 401);

        let mut wrong = authenticated_request("/health/ready");
        wrong
            .headers
            .insert(String::from("authorization"), String::from("Bearer wrong"));
        assert_eq!(service.handle_request(&wrong).0, 401);
        Ok(())
    }

    #[test]
    fn expired_and_under_scoped_credentials_fail_at_the_gateway_boundary() -> Result<(), String> {
        let mut expired = test_service()?;
        expired.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", Some(1), None, "read,mutate,control")?;
        let request = authenticated_request("/v2/instances/instance-1/state");
        let (status, body) = expired.handle_request(&request);
        assert_eq!(status, 401);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "token_expired");

        let mut scoped = test_service()?;
        scoped.config.auth_policy =
            AuthPolicy::test_with_previous("gateway-token", None, None, "read")?;
        let mut action = authenticated_request("/v2/instances/instance-1/action");
        action.method = String::from("POST");
        action.headers.insert(
            String::from("content-type"),
            String::from("application/json"),
        );
        let (status, body) = scoped.handle_request(&action);
        assert_eq!(status, 403);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "insufficient_scope");
        Ok(())
    }

    #[test]
    fn previous_credential_is_accepted_during_gateway_rotation() -> Result<(), String> {
        let mut service = test_service()?;
        service.config.auth_policy = AuthPolicy::test_with_previous(
            "new-gateway-token",
            None,
            Some(("gateway-token", None)),
            "read,mutate,control",
        )?;
        let request = authenticated_request("/v2/instances/instance-1/state");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "sts2.runtime/state_unavailable");
        Ok(())
    }

    #[test]
    fn wrong_instance_is_rejected_even_with_valid_authentication() -> Result<(), String> {
        let mut service = test_service()?;
        let mut request = authenticated_request("/v2/instances/other/state");
        request
            .headers
            .insert(String::from("x-sts2-instance-id"), String::from("other"));
        assert_eq!(service.handle_request(&request).0, 404);

        let request = authenticated_request("/v2/instances/instance-1/state");
        let mut wrong_fence = request;
        wrong_fence
            .headers
            .insert(String::from("x-sts2-lease-epoch"), String::from("2"));
        assert_eq!(service.handle_request(&wrong_fence).0, 409);
        Ok(())
    }

    #[test]
    fn wrong_mcp_session_is_rejected_before_downstream_forwarding() -> Result<(), String> {
        let mut service = test_service()?;
        let mut request = authenticated_request("/v2/instances/instance-1/state");
        request.headers.insert(
            String::from("x-mcp-session-id"),
            String::from("other-mcp-session"),
        );
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 409);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["error_code"], "lease_fence_rejected");
        Ok(())
    }

    #[test]
    fn state_route_accepts_the_typed_mcp_request_body() -> Result<(), String> {
        let mut service = test_service()?;
        let mut request = authenticated_request("/v2/instances/instance-1/state");
        request.body = serde_json::to_vec(&RuntimeV2Message::state_request(
            RuntimeV2Metadata::new(),
            "corr-state",
            "instance-1",
            "session-1",
            "lease-1",
            1,
            0,
        ))
        .map_err(|error| error.to_string())?;
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 503);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["reason"], "downstream_unavailable_before_write");
        Ok(())
    }

    #[test]
    fn metrics_route_is_authenticated_and_reports_queue_capacity() -> Result<(), String> {
        let mut service = test_service()?;
        let request = authenticated_request("/v2/instances/instance-1/metrics");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 200);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["instance_id"], "instance-1");
        assert_eq!(value["queue_capacity"], 8);
        assert_eq!(value["queue_depth"], 0);
        Ok(())
    }

    #[test]
    fn shutdown_route_closes_the_lease_and_marks_admission() -> Result<(), String> {
        let mut service = test_service()?;
        let mut request = authenticated_request("/v2/instances/instance-1/shutdown");
        request.method = String::from("POST");
        let (status, body) = service.handle_request(&request);
        assert_eq!(status, 202);
        let value = serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?;
        assert_eq!(value["status"], "shutdown_requested");
        assert!(service.shutdown_requested);
        assert!(!service.lease_active);
        Ok(())
    }

    #[test]
    fn shutdown_drains_requests_until_admission_producer_exits() -> Result<(), String> {
        use std::net::{TcpListener, TcpStream};
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, mpsc};
        use std::time::Duration;

        let service = test_service()?;
        let metrics = service.metrics.clone();
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let (sender, receiver) = mpsc::sync_channel(2);
        let (finished_sender, finished_receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result =
                super::run_worker(service, receiver, Arc::new(AtomicBool::new(true)), address);
            let _ = finished_sender.send(result);
        });
        let mut shutdown_client = TcpStream::connect(address).map_err(|e| e.to_string())?;
        shutdown_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let (shutdown_stream, _) = listener.accept().map_err(|e| e.to_string())?;
        let mut request = authenticated_request("/v2/instances/instance-1/shutdown");
        request.method = String::from("POST");
        metrics.queue_admitted();
        sender
            .send(super::QueuedRequest {
                stream: shutdown_stream,
                request,
            })
            .map_err(|e| e.to_string())?;
        assert_eq!(
            super::read_response(
                &mut shutdown_client,
                super::Instant::now() + Duration::from_secs(2)
            )
            .map_err(|e| format!("{e:?}"))?
            .status,
            202
        );
        assert!(matches!(
            finished_receiver.recv_timeout(Duration::from_millis(30)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        // Admission was already reading this connection when shutdown began.
        let late_listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let mut late_client =
            TcpStream::connect(late_listener.local_addr().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        late_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let (late_stream, _) = late_listener.accept().map_err(|e| e.to_string())?;
        metrics.queue_admitted();
        sender
            .send(super::QueuedRequest {
                stream: late_stream,
                request: authenticated_request("/v2/instances/instance-1/metrics"),
            })
            .map_err(|e| e.to_string())?;
        drop(sender);
        assert_eq!(
            super::read_response(
                &mut late_client,
                super::Instant::now() + Duration::from_secs(2)
            )
            .map_err(|e| format!("{e:?}"))?
            .status,
            503
        );
        finished_receiver
            .recv_timeout(Duration::from_secs(2))
            .map_err(|e| e.to_string())??;
        worker.join().map_err(|_| String::from("worker panicked"))?;
        assert_eq!(
            metrics.snapshot("instance-1", 2)["cancelled_on_shutdown"],
            1
        );
        assert_eq!(metrics.snapshot("instance-1", 2)["queue_depth"], 0);
        Ok(())
    }
}
