// SPDX-License-Identifier: MIT

use super::*;

pub(crate) struct RuntimeService {
    pub(super) config: RuntimeConfig,
    pub(super) lease_active: bool,
    pub(super) lease_revoked: bool,
    pub(super) allocation_cleanup_lease_id: Option<String>,
    pub(super) shutdown_requested: bool,
    pub(super) runtime_v2: RuntimeV2Ledger<HttpRuntimeV2Forwarder>,
    pub(super) runtime_v3: RuntimeV3GameplayForwarder,
    pub(super) coop_native: CoopNativeForwarder,
    pub(super) coop_native_peer_binding: Option<CoopNativePeerBinding>,
    pub(super) coop_native_pending: Option<CoopNativePendingOperation>,
    pub(super) recovery_catalog: recovery_catalog::RecoveryCatalogCache,
    pub(super) runtime_v4_expert: RuntimeV4ExpertForwarder,
    pub(super) runtime_v4_expert_rest_action: RuntimeV4ExpertRestActionForwarder,
    pub(super) runtime_map: RuntimeMapForwarder,
    pub(super) game_information: GameInformationForwarder,
    pub(super) game_information_capabilities: Option<BoundGameInformationCapabilities>,
    pub(super) game_information_lookup_binding: Option<BoundLookupBinding>,
    pub(super) game_information_live_bootstrap_supported:
        Option<game_information_forwarder::GameInformationProducerAuthority>,
    pub(super) game_information_live_bootstrap_transport_failed: bool,
    pub(super) runtime_v3_baseline: Option<negotiated_capabilities::BoundRuntimeV3Baseline>,
    pub(super) game_information_exchange_timeout: Duration,
    pub(super) game_information_cursor_bindings: BTreeMap<String, Value>,
    pub(super) save_profile: service_save_profile::SaveProfileRuntime,
    pub(super) save_profile_active_run: SaveProfileActiveRun,
    pub(super) seeded_run: SeededRunLedger<HttpSeededRunForwarder>,
    /// Attached approved-profile process lifecycle, or the fail-closed unconfigured default.
    pub(super) process_lifecycle: service_process_lifecycle::ProcessLifecycleRuntime,
    pub(super) journal_path: Option<PathBuf>,
    pub(super) _journal_lock: Option<journal::JournalLock>,
    pub(super) metrics: RuntimeMetrics,
    pub(super) coop_reports: Option<CoopReports>,
    pub(super) recovery: Option<GatewayRecoveryStore>,
    pub(super) recovery_boot: Option<RecoveryBootContext>,
    pub(super) recovery_fence: Option<RecoveryHostFence>,
    pub(super) recovery_lease: Option<RecoveryLease>,
    pub(super) recovery_lease_deadline: Option<Instant>,
    pub(super) recovery_lease_deadline_lease_id: Option<String>,
    pub(super) recovery_host_grant: Option<HostLeaseGrant>,
    /// Negotiated repeated-episode profile, if the caller opted in on a release.
    /// Absence keeps the single-episode permanent-stop default.
    pub(super) episode_profile: Option<episode_profile::EpisodeProfile>,
    pub(super) recovery_clock: recovery_state::RecoveryClock,
    #[cfg(test)]
    pub(super) recovery_test_bootstrap_secret: Option<Vec<u8>>,
}
