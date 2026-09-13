// SPDX-License-Identifier: MIT

use crate::identity::{
    FenceFailure, InstanceId, Lease, LeaseProof, OperationId, Tick, evaluate_fence,
};
use crate::process_identity::{ProcessDescendantIdentity, ProcessIdentity, ProcessLaunch};
use crate::process_profile::LaunchProfile;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProcessFault {
    Unavailable,
    StartRejected,
    InspectionFailed,
    StopFailed,
    StopTimedOut,
    ProfileRequired,
    ProfileNotApproved,
    IdentityMismatch,
    DescendantOutOfScope,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProcessHandle(u64);

impl ProcessHandle {
    /// Creates an opaque process handle owned by the process port.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the opaque process-port value.
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    instance_id: InstanceId,
    profile_id: Option<crate::LaunchProfileId>,
}

impl LaunchSpec {
    pub(crate) const fn new(instance_id: InstanceId) -> Self {
        Self {
            instance_id,
            profile_id: None,
        }
    }

    pub const fn for_profile(instance_id: InstanceId, profile_id: crate::LaunchProfileId) -> Self {
        Self {
            instance_id,
            profile_id: Some(profile_id),
        }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn profile_id(self) -> Option<crate::LaunchProfileId> {
        self.profile_id
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProcessState {
    Running,
    Exited { code: Option<i32> },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StopMode {
    Graceful,
    Force,
}

/// Owns child-process launch, observation, and stop operations for one gateway instance.
pub trait ProcessPort {
    /// Transfers ownership of a started process only on `Ok(handle)`.
    /// On `Err`, no process is transferred: the port must clean up any partial launch before
    /// returning. An adapter unable to guarantee this needs a richer ownership-bearing error
    /// contract before it can implement this seam safely.
    fn start(&mut self, specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault>;

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault>;

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault>;

    /// Starts with a server-resolved profile and transfers an identity-bearing launch.
    ///
    /// A legacy port cannot prove the exact identity or cleanup a partially transferred launch
    /// through this result type. Its default therefore rejects the profile-aware path before
    /// invoking `start`; adapters that can establish and clean up an identity-bearing launch
    /// must override this method. An adapter may report an ambiguous fault after creating a
    /// child (for example when cleanup itself fails); `ProcessLifecycle` treats such a fault as
    /// `Unknown` and retains its durable reservation, then uses `recover_owned` as a read-only
    /// attachment opportunity.
    fn start_with_profile(
        &mut self,
        _specification: LaunchSpec,
        _profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        Err(ProcessFault::ProfileRequired)
    }

    /// Returns the exact identity currently associated with a handle.
    ///
    /// Legacy ports return an intentionally non-matching opaque identity. Profile-aware ports
    /// override this method; lifecycle admission never treats the legacy value as authorized.
    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        match self.inspect(process)? {
            ProcessState::Running => Ok(ProcessIdentity::legacy(process)),
            ProcessState::Exited { .. } => Err(ProcessFault::InspectionFailed),
        }
    }

    /// Returns the currently observed direct descendants of an owned process.
    fn descendants(
        &mut self,
        _process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        Ok(Vec::new())
    }

    /// Recovers a process created for a previously persisted launch intent.
    ///
    /// Returning `None` is a definitive absence only for an adapter that can inspect its owned
    /// process registry; the default reports no recovery capability.
    fn recover_owned(
        &mut self,
        _instance_id: InstanceId,
        _profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        Ok(None)
    }
}

/// Supplies monotonic time; implementations must not use wall time for ordering decisions.
pub trait Clock {
    fn now(&self) -> Tick;
}

/// Decides whether an identity-bearing operation still owns its lease.
pub trait LeaseDecisionPort {
    fn check_fence(
        &mut self,
        current: Option<Lease>,
        target: InstanceId,
        proof: LeaseProof,
        now: Tick,
    ) -> Result<(), FenceFailure>;
}

/// Pure default policy used by the gateway until a persistence-backed policy is authorized.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeterministicLeaseDecision;

impl LeaseDecisionPort for DeterministicLeaseDecision {
    fn check_fence(
        &mut self,
        current: Option<Lease>,
        target: InstanceId,
        proof: LeaseProof,
        now: Tick,
    ) -> Result<(), FenceFailure> {
        evaluate_fence(current.as_ref(), target, proof, now)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    Starting,
    Ready,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthFault {
    Unavailable,
    Malformed,
}

/// Probes readiness and health without giving the gateway game or host authority.
pub trait ReadinessPort {
    fn probe(
        &mut self,
        instance: InstanceId,
        process: ProcessHandle,
    ) -> Result<Readiness, HealthFault>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedRoute {
    ReadOnly,
    Command,
    Receipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportRequest {
    instance_id: InstanceId,
    process: ProcessHandle,
    lease: LeaseProof,
    operation_id: OperationId,
    route: FixedRoute,
    body: Vec<u8>,
}

impl TransportRequest {
    pub(crate) fn new(
        instance_id: InstanceId,
        process: ProcessHandle,
        lease: LeaseProof,
        operation_id: OperationId,
        route: FixedRoute,
        body: Vec<u8>,
    ) -> Self {
        Self {
            instance_id,
            process,
            lease,
            operation_id,
            route,
            body,
        }
    }

    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn process(&self) -> ProcessHandle {
        self.process
    }

    pub const fn lease(&self) -> LeaseProof {
        self.lease
    }

    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub const fn route(&self) -> FixedRoute {
        self.route
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportResponse {
    status: u16,
    body: Vec<u8>,
}

impl TransportResponse {
    /// Creates an opaque bounded response from the downstream transport seam.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    pub const fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn body_len(&self) -> usize {
        self.body.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportFault {
    Unavailable,
    Disconnected,
    Rejected,
}

/// Forwards only gateway-validated, fixed-route requests to the selected process.
pub trait TransportPort {
    fn forward(&mut self, request: TransportRequest) -> Result<TransportResponse, TransportFault>;
}
