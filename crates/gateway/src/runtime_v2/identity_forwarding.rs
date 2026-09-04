// SPDX-License-Identifier: MIT

/// Deterministic identity-fence failures for the v2 ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2FenceFailure {
    WrongInstance,
    WrongSession,
    WrongLease,
    StaleEpoch,
}

impl fmt::Display for RuntimeV2FenceFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::WrongInstance => "runtime-v2 instance identity mismatch",
            Self::WrongSession => "runtime-v2 session identity mismatch",
            Self::WrongLease => "runtime-v2 lease identity mismatch",
            Self::StaleEpoch => "runtime-v2 lease epoch is stale",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for RuntimeV2FenceFailure {}

/// Bounded operation-store configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeV2LedgerConfig {
    operation_capacity: usize,
}

impl RuntimeV2LedgerConfig {
    #[must_use]
    pub const fn new(operation_capacity: usize) -> Self {
        Self { operation_capacity }
    }

    pub const fn operation_capacity(self) -> usize {
        self.operation_capacity
    }
}

/// A fixed action dispatch request presented to the gateway-owned forwarding seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2ForwardRequest {
    message: RuntimeV2Message,
}

impl RuntimeV2ForwardRequest {
    #[must_use]
    pub fn new(message: RuntimeV2Message) -> Self {
        Self { message }
    }

    pub fn message(&self) -> &RuntimeV2Message {
        &self.message
    }
}

/// A read-only retained-receipt lookup request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2ReceiptRequest {
    message: RuntimeV2Message,
    key: RuntimeV2OperationKey,
    action: RuntimeV2Action,
}

impl RuntimeV2ReceiptRequest {
    fn new(message: RuntimeV2Message, key: RuntimeV2OperationKey, action: RuntimeV2Action) -> Self {
        Self {
            message,
            key,
            action,
        }
    }

    pub fn message(&self) -> &RuntimeV2Message {
        &self.message
    }

    pub fn key(&self) -> &RuntimeV2OperationKey {
        &self.key
    }

    pub fn action(&self) -> &RuntimeV2Action {
        &self.action
    }
}

/// One gateway-retained operation serialized for restart recovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2PersistedOperation {
    pub request: RuntimeV2Message,
    pub result: Option<RuntimeV2Message>,
}

/// The gateway-owned Runtime-v2 state needed to reconstruct a ledger after restart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2PersistedState {
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub observation: RuntimeV2Observation,
    pub operations: Vec<RuntimeV2PersistedOperation>,
}

/// Transport outcomes distinguish pre-write failure from uncertainty after a write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2TransportFault {
    UnavailableBeforeWrite,
    RejectedBeforeWrite,
    TimeoutBeforeWrite,
    DisconnectedBeforeWrite,
    TimeoutAfterWrite,
    DisconnectedAfterWrite,
    MalformedResponse,
    ReceiptUnavailable,
}

/// The only forwarding interface accepted by the Runtime-v2 ledger.
pub trait RuntimeV2ForwardingPort {
    /// Dispatches one action request. Implementations must not resend it internally.
    fn forward_runtime_v2(
        &mut self,
        request: RuntimeV2ForwardRequest,
    ) -> Result<RuntimeV2Message, RuntimeV2TransportFault>;

    /// Reads a retained receipt. This method must never authorize or apply mutation.
    fn read_runtime_v2_receipt(
        &mut self,
        request: RuntimeV2ReceiptRequest,
    ) -> Result<Option<RuntimeV2Message>, RuntimeV2TransportFault>;
}

/// Errors that stop a v2 request before any mutation-bearing dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeV2LedgerError {
    InvalidRequest(RuntimeV2ValidationError),
    RequestDigest(RuntimeV2CodecError),
    MissingOperationId,
    CapacityExceeded,
    OperationNotFound,
    OperationInProgress,
    Fence(RuntimeV2FenceFailure),
    StaleGeneration { expected: u64, actual: u64 },
    ZeroCapacity,
    PersistedStateMismatch,
    PersistedStateInvalid,
    PersistenceFailed,
}

impl fmt::Display for RuntimeV2LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(error) => write!(formatter, "invalid Runtime-v2 request: {error}"),
            Self::RequestDigest(error) => {
                write!(formatter, "Runtime-v2 request digest failed: {error}")
            }
            Self::MissingOperationId => formatter.write_str("Runtime-v2 operation_id is required"),
            Self::CapacityExceeded => formatter.write_str("Runtime-v2 operation store is full"),
            Self::OperationNotFound => formatter.write_str("Runtime-v2 operation was not retained"),
            Self::OperationInProgress => {
                formatter.write_str("Runtime-v2 operation is still being dispatched")
            }
            Self::Fence(error) => write!(formatter, "Runtime-v2 request fenced: {error}"),
            Self::StaleGeneration { expected, actual } => write!(
                formatter,
                "Runtime-v2 generation is stale: expected {expected}, received {actual}"
            ),
            Self::ZeroCapacity => {
                formatter.write_str("Runtime-v2 operation capacity must be positive")
            }
            Self::PersistedStateMismatch => {
                formatter.write_str("persisted Runtime-v2 state belongs to another lease")
            }
            Self::PersistedStateInvalid => {
                formatter.write_str("persisted Runtime-v2 state is invalid")
            }
            Self::PersistenceFailed => {
                formatter.write_str("Runtime-v2 durable state could not be written")
            }
        }
    }
}

impl std::error::Error for RuntimeV2LedgerError {}

/// Errors that can occur while serializing a canonical request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2CodecError {
    Serialization,
}

impl fmt::Display for RuntimeV2CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Runtime-v2 JSON serialization failed")
    }
}

impl std::error::Error for RuntimeV2CodecError {}
