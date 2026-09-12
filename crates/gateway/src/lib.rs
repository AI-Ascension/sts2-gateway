// SPDX-License-Identifier: MIT

mod config;
mod control;
mod coop_session;
mod exact_checkpoint_reference;
mod fencing;
mod forwarding;
mod identity;
mod lifecycle;
mod maintenance;
mod ports;
mod process_supervisor;
mod protocol_artifact;
mod recovery;
mod runtime_v2;
mod runtime_v2_artifact;
mod seeded_run;

use std::fmt;

pub use config::{ConfigError, GatewayConfig};

pub use coop_session::{CoopPeerRole, CoopSession, CoopSessionError, CoopSynchronizationSnapshot};
pub use exact_checkpoint_reference::{
    ASSURANCE_LABELS, HANDLE_PREFIX, MAX_BOUNDARY_LABEL_BYTES, REFERENCE_CONTRACT,
    REFERENCE_SCHEMA, REFERENCE_SCHEMA_DIGEST, REFERENCE_VERSION, ReferenceError,
    validate_exact_checkpoint_reference, verify_exact_checkpoint_reference_artifact,
};
pub use identity::{
    CallerId, FenceFailure, InstanceId, Lease, LeaseEpoch, LeaseId, LeaseProof, OperationId,
    SessionId, Tick, evaluate_fence,
};
pub use lifecycle::{InstanceSnapshot, LifecycleState};
pub use ports::{
    Clock, DeterministicLeaseDecision, FixedRoute, HealthFault, LaunchSpec, LeaseDecisionPort,
    ProcessFault, ProcessHandle, ProcessPort, ProcessState, Readiness, ReadinessPort, StopMode,
    TransportFault, TransportPort, TransportRequest, TransportResponse,
};
pub use process_supervisor::{
    ProcessSupervisor, ProcessSupervisorConfig, ProcessSupervisorConfigError,
    ProcessSupervisorError,
};
pub use protocol_artifact::{
    ArtifactError, POC_ARTIFACT, POC_GENERATOR, POC_MAX_SETTLED_EFFECTS, POC_MAX_UNITS,
    POC_PROTOCOL_VERSION, POC_SCHEMA_DIGEST, POC_SCHEMA_SOURCE, verify_poc_artifact,
};
pub use recovery::{
    GatewayRecoveryStore, HOST_LEASE_CONTROL_CONTRACT, HOST_LEASE_CONTROL_SCHEMA_DIGEST,
    MAX_HOST_LEASE_FRAME_BYTES, MAX_HOST_LEASE_PAYLOAD_BYTES, MAX_HOST_LEASE_PROOF_BYTES,
    MAX_RECOVERY_ACTION_BYTES, MAX_RECOVERY_FRAME_BYTES, MAX_RECOVERY_RESPONSE_BYTES,
    MAX_WIRE_INTEGER, RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST,
    RECOVERY_TOMBSTONE_RETENTION_MILLIS, RUNTIME_V3_SCHEMA_DIGEST, RecoveryAdmissionTicket,
    RecoveryBootContext, RecoveryBootState, RecoveryEffectWitness, RecoveryHostFence,
    RecoveryHostLeaseBinding, RecoveryHostLeaseState, RecoveryIntentResult, RecoveryLease,
    RecoveryLeaseProof, RecoveryLeaseRequest, RecoveryLeaseState, RecoveryOperation,
    RecoveryOperationIntent, RecoveryOperationState, RecoveryReleaseSet, RecoveryStoreConfig,
    RecoveryStoreError, RecoveryStorePragmas, RecoveryTicketState, RecoveryUncertaintyReason,
    canonical_json_digest, canonicalize_recovery_action, sha256_hex,
};
pub use runtime_v2::{
    RuntimeV2Action, RuntimeV2Authority, RuntimeV2Binding, RuntimeV2CodecError,
    RuntimeV2CombatPhase, RuntimeV2EffectWitness, RuntimeV2FenceFailure, RuntimeV2ForwardRequest,
    RuntimeV2ForwardingPort, RuntimeV2Ledger, RuntimeV2LedgerConfig, RuntimeV2LedgerError,
    RuntimeV2Message, RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation,
    RuntimeV2OperationKey, RuntimeV2PersistedOperation, RuntimeV2PersistedState,
    RuntimeV2Provenance, RuntimeV2ReceiptRequest, RuntimeV2ReceiptRetention,
    RuntimeV2RecoveryCapabilities, RuntimeV2RecoveryContract, RuntimeV2RecoveryError,
    RuntimeV2RecoveryFailureDomain, RuntimeV2RequestDigest, RuntimeV2Status,
    RuntimeV2TransportFault, RuntimeV2ValidationError,
};
pub use runtime_v2_artifact::{
    RUNTIME_V2_ACTION_ID, RUNTIME_V2_ARTIFACT, RUNTIME_V2_EFFECT_KIND, RUNTIME_V2_GENERATOR,
    RUNTIME_V2_MAX_GENERATION, RUNTIME_V2_MAX_TURN_INDEX, RUNTIME_V2_PLAYER_TURN_PHASE,
    RUNTIME_V2_PROTOCOL_VERSION, RUNTIME_V2_SCHEMA_DIGEST, RUNTIME_V2_SCHEMA_SOURCE,
    RuntimeV2ArtifactError, RuntimeV2ArtifactFile, RuntimeV2ArtifactFiles,
    runtime_v2_artifact_files, verify_runtime_v2_artifact, verify_runtime_v2_artifact_files,
};
pub use seeded_run::{
    SEEDED_RUN_ARTIFACT, SEEDED_RUN_EFFECT_KIND, SEEDED_RUN_GENERATOR, SEEDED_RUN_MAX_ACTS,
    SEEDED_RUN_MAX_CONTEXT_ID_BYTES, SEEDED_RUN_MAX_CONTEXT_TEXT_BYTES, SEEDED_RUN_MAX_GENERATION,
    SEEDED_RUN_MAX_IDENTITY_BYTES, SEEDED_RUN_MAX_MODIFIERS, SEEDED_RUN_MAX_SEED_BYTES,
    SEEDED_RUN_PROTOCOL_VERSION, SEEDED_RUN_SCHEMA_DIGEST, SEEDED_RUN_SCHEMA_SOURCE,
    SeededRunBinding, SeededRunCharacter, SeededRunCompatibility, SeededRunContext,
    SeededRunEffectWitness, SeededRunForwardRequest, SeededRunForwardingPort, SeededRunGameMode,
    SeededRunIdentityDigest, SeededRunLedger, SeededRunLedgerConfig, SeededRunLedgerError,
    SeededRunMessage, SeededRunMessageKind, SeededRunMode, SeededRunObservation,
    SeededRunPersistedOperation, SeededRunPersistedState, SeededRunProfileBaseline,
    SeededRunProfileKind, SeededRunProvenance, SeededRunReceiptRequest, SeededRunSavePolicy,
    SeededRunSelectionContext, SeededRunStatus, SeededRunTransportFault, SeededRunValidationError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayError {
    AdmissionClosed,
    CapacityExceeded,
    IdentityExhausted,
    InstanceNotFound,
    IdentityMismatch,
    InvalidState(LifecycleState),
    Fence(FenceFailure),
    ProcessStart(ProcessFault),
    ProcessInspection(ProcessFault),
    ProcessCrashed,
    ProcessStop(ProcessFault),
    Readiness(HealthFault),
    BodyTooLarge { limit: usize, actual: usize },
    ResponseTooLarge { limit: usize, actual: usize },
    Transport(TransportFault),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdmissionClosed => formatter.write_str("gateway admission is closed"),
            Self::CapacityExceeded => formatter.write_str("gateway capacity is exhausted"),
            Self::IdentityExhausted => formatter.write_str("gateway identity space is exhausted"),
            Self::InstanceNotFound => formatter.write_str("instance was not found"),
            Self::IdentityMismatch => formatter.write_str("caller or session identity mismatched"),
            Self::InvalidState(state) => {
                write!(formatter, "operation is invalid in state {state:?}")
            }
            Self::Fence(failure) => write!(formatter, "lease fence rejected request: {failure:?}"),
            Self::ProcessStart(fault) => write!(formatter, "process start failed: {fault:?}"),
            Self::ProcessInspection(fault) => {
                write!(formatter, "process inspection failed: {fault:?}")
            }
            Self::ProcessCrashed => formatter.write_str("owned process exited unexpectedly"),
            Self::ProcessStop(fault) => write!(formatter, "process stop failed: {fault:?}"),
            Self::Readiness(fault) => write!(formatter, "readiness probe failed: {fault:?}"),
            Self::BodyTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "request body has {actual} bytes; limit is {limit}"
                )
            }
            Self::ResponseTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "response body has {actual} bytes; limit is {limit}"
                )
            }
            Self::Transport(fault) => write!(formatter, "downstream transport failed: {fault:?}"),
        }
    }
}

impl std::error::Error for GatewayError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Allocation {
    instance_id: InstanceId,
    lease: Lease,
}

impl Allocation {
    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn lease(self) -> Lease {
        self.lease
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupResult {
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupStatus {
    Cleaned,
    Failed(ProcessFault),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpirationEvent {
    instance_id: InstanceId,
    status: CleanupStatus,
}

impl ExpirationEvent {
    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn status(self) -> CleanupStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShutdownReport {
    stopped: usize,
    failed: usize,
}

impl ShutdownReport {
    pub const fn stopped(self) -> usize {
        self.stopped
    }

    pub const fn failed(self) -> usize {
        self.failed
    }
}

pub struct Gateway<C, P, R, T, F = DeterministicLeaseDecision> {
    pub(crate) config: GatewayConfig,
    pub(crate) clock: C,
    pub(crate) process: P,
    pub(crate) readiness: R,
    pub(crate) transport: T,
    pub(crate) fence: F,
    pub(crate) instances: std::collections::BTreeMap<InstanceId, lifecycle::InstanceRecord>,
    pub(crate) next_instance_id: u64,
    pub(crate) next_lease_id: u64,
    pub(crate) is_admitting: bool,
}

impl<C, P, R, T> Gateway<C, P, R, T, DeterministicLeaseDecision> {
    /// Creates a control plane with the pure default lease-fence policy.
    pub fn new(config: GatewayConfig, clock: C, process: P, readiness: R, transport: T) -> Self {
        Self::with_fence(
            config,
            clock,
            process,
            readiness,
            transport,
            DeterministicLeaseDecision,
        )
    }
}

impl<C, P, R, T, F> Gateway<C, P, R, T, F> {
    /// Creates a control plane around explicitly injected lifecycle and boundary ports.
    pub fn with_fence(
        config: GatewayConfig,
        clock: C,
        process: P,
        readiness: R,
        transport: T,
        fence: F,
    ) -> Self {
        Self {
            config,
            clock,
            process,
            readiness,
            transport,
            fence,
            instances: std::collections::BTreeMap::new(),
            next_instance_id: 1,
            next_lease_id: 1,
            is_admitting: true,
        }
    }
}
