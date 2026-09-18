// SPDX-License-Identifier: MIT

//! Attached-runtime composition for the approved-profile process lifecycle.
//!
//! Two things are deliberately absent from this slice, and their absence is the fail-closed
//! default rather than a gap to be filled by invention:
//!
//! 1. **No concrete OS process adapter.** ADR 0024 places executable/signal/host behavior behind
//!    the injected `ProcessPort` and records it as a reviewed deployment input. This module
//!    therefore never constructs a process, resolves an executable path, or reads a
//!    caller-supplied command. An unconfigured deployment reports the surface unavailable and
//!    refuses every submission before any effect.
//! 2. **No caller-supplied identity.** The launch-profile catalog is built only from the
//!    gateway's own configuration. A caller supplies an opaque `LaunchProfileId` and nothing
//!    else; executable, install, image, user-data namespace, and process policy all come from the
//!    configured catalog.
//!
//! The HTTP surface itself lives in `service_process_lifecycle_requests.rs`; this module owns only
//! the composition state and the startup validation an operator can fail on.

use std::sync::Arc;
use std::time::Instant;

use sts2_gateway::{
    AuthorityEpoch, Clock, LeaseDecisionPort, LifecycleRecordStore, ProcessLifecycle,
    ProcessLifecycleConfig, ProcessPort, SqliteLifecycleStore, Tick,
};

use super::service_process_lifecycle_config::ProcessLifecycleDeployment;
use super::service_process_lifecycle_fence::AttachedLeaseFence;

/// The attached adapter has no independent lease deadline.
///
/// Liveness is the HTTP gate's decision (`service_lease.rs`), which runs on every request before
/// dispatch and rejects an inactive, revoked, or shut-down lease with full request context. This
/// adapter states that absence explicitly rather than inventing a deadline that would expire a
/// live lease on a timer nobody configured.
///
/// `AttachedLeaseFence` never reads this value, so it is not a timeout an operator can tune; it
/// exists only because `Lease` carries the field. Changing it cannot make a lease expire, and
/// `liveness_is_decided_by_the_http_gate_not_the_lifecycle_fence` pins that split.
pub(super) const NO_LEASE_EXPIRY: Tick = Tick::from_millis(u64::MAX);

/// The boxed port set the attached runtime composes at startup.
pub(super) type BoxedProcessLifecycle = ProcessLifecycle<
    Box<dyn Clock + Send>,
    Box<dyn ProcessPort + Send>,
    Box<dyn LifecycleRecordStore + Send>,
    Box<dyn LeaseDecisionPort + Send>,
>;

/// Monotonic clock over the runtime's own start instant.
///
/// The coordinator orders fence decisions with this value, so it must never move backwards and
/// must never be wall time.
pub(super) struct AttachedClock {
    started: Instant,
}

impl AttachedClock {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Clock for AttachedClock {
    fn now(&self) -> Tick {
        let elapsed = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        Tick::from_millis(elapsed)
    }
}

/// The attached runtime's lifecycle state.
///
/// Three states rather than two, because "no reviewed process adapter is installed" and "no
/// configuration was supplied" are different operator facts and must not be reported alike.
///
/// `Ready` is unreachable in the current shipped binary because ADR 0024 records the concrete OS
/// process adapter as a reviewed deployment input that this build does not install. The variant
/// and its composition path exist so the adapter is a composition change rather than a rewrite,
/// and the focused tests exercise them at this commit. This is the same test-exercised,
/// production-unwired seam pattern already used for `handle_request` in `service_routes.rs`.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) enum ProcessLifecycleRuntime {
    /// No lifecycle configuration was supplied; every effect is refused.
    Unconfigured,
    /// Configuration was validated, but no reviewed OS process adapter is installed.
    ///
    /// ADR 0024 records the concrete executable/signal/host adapter as a reviewed deployment
    /// input. Until one is installed, the surface refuses every effect while still reporting the
    /// exact profiles an operator configured, so a misconfiguration is visible rather than silent.
    Configured { profiles: Vec<u64> },
    /// Composed coordinator plus the clock its fence decisions read and the advertised ids.
    ///
    /// The coordinator is boxed so the variant that carries it stays pointer-sized; this state is
    /// inspected on every request, including the `Unconfigured` and `Configured` cases that must
    /// not pay for a large variant.
    Ready {
        lifecycle: Box<BoxedProcessLifecycle>,
        profiles: Vec<u64>,
    },
}

impl ProcessLifecycleRuntime {
    pub(super) const fn unconfigured() -> Self {
        Self::Unconfigured
    }

    /// Validates a deployment configuration without composing a coordinator.
    ///
    /// Used by the production entry point while the concrete process adapter remains a reviewed
    /// deployment input: the catalog is built and every profile is checked, so a bad profile
    /// fails startup instead of surfacing as a runtime rejection.
    pub(super) fn configured(deployment: &ProcessLifecycleDeployment) -> Result<Self, String> {
        let (_, profiles) =
            super::service_process_lifecycle_config::build_catalog(&deployment.profiles)?;
        // Validate the capacity budget and the store location at startup. A deployment that could
        // not persist operations, or whose budget the coordinator would reject, must fail here
        // rather than after a caller has submitted an operation.
        ProcessLifecycleConfig::try_new_with_record_budget(
            deployment.max_processes,
            deployment.max_records,
        )
        .map_err(|error| format!("process-lifecycle capacity is invalid: {error:?}"))?;
        deployment.validate_store()?;
        Ok(Self::Configured { profiles })
    }

    /// Composes the coordinator from a deployment configuration and injected ports.
    ///
    /// The durable store is opened and integrity-checked before the coordinator is built, so a
    /// deployment that cannot persist operations fails startup instead of accepting an operation
    /// it could not reconcile.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn compose(
        deployment: ProcessLifecycleDeployment,
        process: Box<dyn ProcessPort + Send>,
    ) -> Result<Self, String> {
        let (catalog, profiles) =
            super::service_process_lifecycle_config::build_catalog(&deployment.profiles)?;
        let store = SqliteLifecycleStore::open(deployment.store_path())
            .map_err(|error| format!("process-lifecycle store open failed: {error:?}"))?;
        let config = ProcessLifecycleConfig::try_new_with_record_budget(
            deployment.max_processes,
            deployment.max_records,
        )
        .map_err(|error| format!("process-lifecycle capacity is invalid: {error:?}"))?;
        let clock = Arc::new(AttachedClock::new());
        let lifecycle = ProcessLifecycle::new(
            config,
            catalog,
            Box::new(Arc::clone(&clock)) as Box<dyn Clock + Send>,
            process,
            Box::new(store) as Box<dyn LifecycleRecordStore + Send>,
            // Composed here rather than injected: `AttachedLeaseFence` is the only fence whose
            // liveness rule matches this adapter, so leaving it injectable would allow a caller to
            // compose a coordinator that disagrees with the HTTP gate about lease validity.
            Box::new(AttachedLeaseFence) as Box<dyn LeaseDecisionPort + Send>,
        )
        .map_err(|error| format!("process lifecycle is invalid: {error:?}"))?;
        Ok(Self::Ready {
            lifecycle: Box::new(lifecycle),
            profiles,
        })
    }

    pub(super) const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Approved profile ids in ascending order, or `None` when no coordinator is composed.
    pub(super) fn profiles(&self) -> Option<&[u64]> {
        match self {
            Self::Unconfigured => None,
            Self::Ready { profiles, .. } => Some(profiles),
            Self::Configured { profiles, .. } => Some(profiles),
        }
    }

    /// Current authority epoch, or `None` when no coordinator is composed.
    pub(super) fn authority_epoch(
        &self,
        instance_id: sts2_gateway::InstanceId,
    ) -> Option<AuthorityEpoch> {
        match self {
            Self::Unconfigured | Self::Configured { .. } => None,
            Self::Ready { lifecycle, .. } => Some(lifecycle.current_authority_epoch(instance_id)),
        }
    }

    pub(super) fn lifecycle_mut(&mut self) -> Option<&mut BoxedProcessLifecycle> {
        match self {
            Self::Unconfigured | Self::Configured { .. } => None,
            Self::Ready { lifecycle, .. } => Some(lifecycle),
        }
    }
}
