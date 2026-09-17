// SPDX-License-Identifier: MIT

//! The spawned `sts2-gateway-runtime` process and its temporary durable store.
//!
//! Every identity below is the identity the *served* process is configured
//! with, so a test asserts on decisions the gateway made for itself rather than
//! on values injected into an in-process fixture.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::wire;

/// Deployment identity the spawned gateway binds its durable authority to.
pub(crate) const DEPLOYMENT: &str = "00000000-0000-4000-8000-000000000001";
/// Instance identity the spawned gateway serves.
pub(crate) const INSTANCE: &str = "00000000-0000-4000-8000-000000000002";
/// Instance incarnation carried in the bootstrap request.
pub(crate) const INCARNATION: &str = "00000000-0000-4000-8000-000000000003";
/// Recovery principal allowed to drive the recovery routes.
pub(crate) const CALLER: &str = "00000000-0000-4000-8000-00000000000a";
/// Session identity the spawned gateway fences mutations against.
pub(crate) const SESSION: &str = "00000000-0000-4000-8000-00000000000b";
/// MCP session identity the spawned gateway fences mutations against.
pub(crate) const MCP_SESSION: &str = "00000000-0000-4000-8000-00000000000c";
/// Boundary state identity used by the episode-one operation.
pub(crate) const STATE_ID: &str = "00000000-0000-4000-8000-000000000003";
/// Observed legal-action catalogue digest used by the episode-one operation.
pub(crate) const CATALOG_DIGEST: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// Operation identity of the episode-one receipt. Must be a UUIDv4.
pub(crate) const EPISODE_ONE_OPERATION: &str = "00000000-0000-4000-8000-000000000010";
/// Lease policy the gateway is configured with.
pub(crate) const TTL_SECONDS: u64 = 30;
/// Renewal policy the gateway is configured with. Thirty seconds is longer than
/// any single test, so no episode needs a renewal to stay live.
pub(crate) const RENEWAL_INTERVAL_SECONDS: u64 = 10;

/// Normal gateway credential. Authenticates the gameplay release route.
pub(crate) const GATEWAY_TOKEN: &str = "gateway-token";
/// Recovery credential. Recovery routes authorize through `authorize_recovery`.
pub(crate) const RECOVERY_TOKEN: &str = "recovery-token";
/// Downstream control credential for the host hop.
pub(crate) const MOD_TOKEN: &str = "mod-token";
/// Host principal the gateway expects on host lease-control acknowledgments.
pub(crate) const HOST_PRINCIPAL: &str = "00000000-0000-4000-8000-00000000000e";
/// Host lease-control key: bytes `0x00..=0x1f`. The host fake and the served
/// gateway must derive the same acknowledgment proof from it.
pub(crate) const HOST_LEASE_KEY: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
/// Bootstrap secret: bytes `0x10..=0x2f`, used for the gateway-to-host proof.
const BOOTSTRAP_SECRET: [u8; 32] = [
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47,
];
/// Attached-loopback lease identity. The recovery profile replaces it with the
/// durable lease, so it only has to be a safe identity.
const ATTACHED_LEASE_ID: &str = "00000000-0000-4000-8000-00000000000d";

const READY_DEADLINE: Duration = Duration::from_secs(20);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(25);
const DIAGNOSTIC_LIMIT: usize = 4_096;

/// A recovery store path unique to one test, removed when the test ends.
pub(crate) struct TempStore {
    path: PathBuf,
}

impl TempStore {
    pub(crate) fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sts2-real-process-episode-{label}-{}-{nanos}-{sequence}.db",
            std::process::id()
        ));
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("gateway-recovery.lock"));
        let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
    }
}

/// A served gateway process. The child is killed on drop as a backstop so a
/// failing assertion cannot leave an orphan listening on the port.
pub(crate) struct GatewayProcess {
    child: Child,
    address: String,
    drains: Vec<JoinHandle<()>>,
    diagnostics: Arc<Mutex<String>>,
}

impl GatewayProcess {
    /// Spawns the built `sts2-gateway-runtime` binary against `store_path`.
    ///
    /// The child inherits no ambient `STS2_*` configuration, so the served
    /// process cannot accidentally satisfy an assertion from the test's own
    /// environment.
    pub(crate) fn spawn(host_address: &str, store_path: &Path) -> Result<Self, String> {
        let port = free_loopback_port()?;
        let address = format!("127.0.0.1:{port}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_sts2-gateway-runtime"))
            .env_clear()
            .env("STS2_GATEWAY_ADDR", &address)
            .env("STS2_MOD_ADDR", host_address)
            .env("STS2_GATEWAY_TOKEN", GATEWAY_TOKEN)
            .env("STS2_RECOVERY_TOKEN", RECOVERY_TOKEN)
            .env("STS2_MOD_TOKEN", MOD_TOKEN)
            .env("STS2_INSTANCE_ID", INSTANCE)
            .env("STS2_CALLER_ID", CALLER)
            .env("STS2_SESSION_ID", SESSION)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION)
            .env("STS2_LEASE_ID", ATTACHED_LEASE_ID)
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_DEPLOYMENT_ID", DEPLOYMENT)
            .env("STS2_RECOVERY_STORE", store_path)
            .env("STS2_RUNTIME_HOST_PRINCIPAL_ID", HOST_PRINCIPAL)
            .env(
                "STS2_RUNTIME_HOST_LEASE_KEY",
                STANDARD.encode(HOST_LEASE_KEY),
            )
            .env(
                "STS2_RUNTIME_BOOTSTRAP_SECRET",
                STANDARD.encode(BOOTSTRAP_SECRET),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to spawn sts2-gateway-runtime: {error}"))?;

        let mut drains = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            drains.push(drain(stdout, Arc::new(Mutex::new(String::new()))));
        }
        let diagnostics = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            drains.push(drain(stderr, Arc::clone(&diagnostics)));
        }
        Ok(Self {
            child,
            address,
            drains,
            diagnostics,
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    /// Waits until the served process answers an authenticated liveness probe.
    ///
    /// The readiness route itself stays `503` until a boot is fenced, so the
    /// probe is `/health/live`: it proves the listener, the request parser, and
    /// the auth policy are serving, which is what "the process is up" means
    /// before the first bootstrap.
    pub(crate) fn await_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "gateway process exited before it served a probe ({status}): {}",
                    self.diagnostic_tail()
                ));
            }
            if wire::get_status(&self.address, "/health/live", GATEWAY_TOKEN)
                .is_ok_and(|s| s == 200)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "gateway process never answered a liveness probe: {}",
                    self.diagnostic_tail()
                ));
            }
            std::thread::sleep(READY_POLL_INTERVAL);
        }
    }

    pub(crate) fn terminate(&mut self) -> Result<(), String> {
        let _ = self.child.kill();
        let status = self
            .child
            .wait()
            .map_err(|error| format!("failed to reap the gateway process: {error}"))?;
        self.finish_drains();
        if status.success() {
            return Err(String::from(
                "the gateway process exited cleanly; a killed process was expected",
            ));
        }
        Ok(())
    }

    fn finish_drains(&mut self) {
        for drain in self.drains.drain(..) {
            let _ = drain.join();
        }
    }

    /// The retained stderr tail. Bounded so a chatty child cannot inflate a
    /// failure message.
    fn diagnostic_tail(&self) -> String {
        self.diagnostics
            .lock()
            .map_or_else(|_| String::from("<unavailable>"), |tail| tail.clone())
    }
}

impl Drop for GatewayProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.finish_drains();
    }
}

/// Reserves a loopback port, then releases it for the child to bind. The
/// gateway rejects port `0`, so the port has to be chosen here.
fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to reserve a loopback port: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("failed to read the reserved port: {error}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn drain(mut reader: impl Read + Send + 'static, sink: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 2048];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    let Ok(mut sink) = sink.lock() else {
                        return;
                    };
                    sink.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if sink.len() > DIAGNOSTIC_LIMIT {
                        let cut = sink.len() - DIAGNOSTIC_LIMIT;
                        *sink = sink.chars().skip(cut).collect();
                    }
                }
            }
        }
    })
}
