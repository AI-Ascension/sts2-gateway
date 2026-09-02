// SPDX-License-Identifier: MIT

/// Deterministic validation failures for Runtime-v2 values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2ValidationError {
    Metadata,
    Provenance,
    InvalidIdentity,
    GenerationBounds,
    ObservationBounds,
    ActionBounds,
    EffectBounds,
    ResultShape,
}

impl fmt::Display for RuntimeV2ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Metadata => "runtime-v2 metadata is unsupported",
            Self::Provenance => "runtime-v2 provenance is unsupported",
            Self::InvalidIdentity => "runtime-v2 identity is empty, unsafe, or too long",
            Self::GenerationBounds => "runtime-v2 generation is outside the bound",
            Self::ObservationBounds => "runtime-v2 observation is outside the bound",
            Self::ActionBounds => "runtime-v2 action is outside the fixed action identity",
            Self::EffectBounds => "runtime-v2 effect witness is outside the fixed identity",
            Self::ResultShape => "runtime-v2 message fields do not match the message kind/status",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for RuntimeV2ValidationError {}

/// A stable bounded key for one operation context.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuntimeV2OperationKey {
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    operation_id: String,
}

impl RuntimeV2OperationKey {
    /// Creates the key scoped by instance, session, lease, epoch, and operation.
    #[must_use]
    pub fn new(
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        operation_id: &str,
    ) -> Self {
        Self {
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
            operation_id: operation_id.to_owned(),
        }
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

/// A digest of the complete canonical request identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuntimeV2RequestDigest(u64);

impl RuntimeV2RequestDigest {
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RuntimeV2RequestDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:016x}", self.0)
    }
}

/// Runtime-v2 identity and the gateway's last known observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2Binding {
    metadata: RuntimeV2Metadata,
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    observation: RuntimeV2Observation,
}

impl RuntimeV2Binding {
    /// Creates a binding using the owner-local Runtime-v2 metadata.
    pub fn new(
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        observation: RuntimeV2Observation,
    ) -> Result<Self, RuntimeV2ValidationError> {
        Self::with_metadata(
            RuntimeV2Metadata::new(),
            instance_id,
            session_id,
            lease_id,
            lease_epoch,
            observation,
        )
    }

    /// Creates a binding with explicit metadata for artifact-boundary tests.
    pub fn with_metadata(
        metadata: RuntimeV2Metadata,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        observation: RuntimeV2Observation,
    ) -> Result<Self, RuntimeV2ValidationError> {
        metadata.validate()?;
        validate_identity(instance_id)?;
        validate_identity(session_id)?;
        validate_identity(lease_id)?;
        if lease_epoch > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2ValidationError::GenerationBounds);
        }
        observation.validate()?;
        Ok(Self {
            metadata,
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
            observation,
        })
    }

    pub fn metadata(&self) -> &RuntimeV2Metadata {
        &self.metadata
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    pub const fn observation(&self) -> RuntimeV2Observation {
        self.observation
    }

    fn fence_failure(&self, request: &RuntimeV2Message) -> Option<RuntimeV2FenceFailure> {
        if request.instance_id != self.instance_id {
            return Some(RuntimeV2FenceFailure::WrongInstance);
        }
        if request.session_id != self.session_id {
            return Some(RuntimeV2FenceFailure::WrongSession);
        }
        if request.lease_id != self.lease_id {
            return Some(RuntimeV2FenceFailure::WrongLease);
        }
        if request.lease_epoch != self.lease_epoch {
            return Some(RuntimeV2FenceFailure::StaleEpoch);
        }
        None
    }
}
