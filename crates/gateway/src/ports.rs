// SPDX-License-Identifier: MIT

use crate::identity::{
    FenceFailure, InstanceId, Lease, LeaseProof, OperationId, Tick, evaluate_fence,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessFault {
    Unavailable,
    StartRejected,
    InspectionFailed,
    StopFailed,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchSpec {
    instance_id: InstanceId,
}

impl LaunchSpec {
    pub(crate) const fn new(instance_id: InstanceId) -> Self {
        Self { instance_id }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Running,
    Exited { code: Option<i32> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopMode {
    Graceful,
    Force,
}

/// Owns child-process launch, observation, and stop operations for one gateway instance.
pub trait ProcessPort {
    fn start(&mut self, specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault>;

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault>;

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault>;
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
