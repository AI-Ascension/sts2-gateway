// SPDX-License-Identifier: MIT

/// A bounded combat phase used by the Runtime-v2 observation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RuntimeV2CombatPhase {
    #[serde(rename = "outside_combat")]
    OutsideCombat,
    #[serde(rename = "combat/player_turn")]
    PlayerTurn,
    #[serde(rename = "combat/enemy_turn")]
    EnemyTurn,
}

/// The five operation outcomes in the frozen Runtime-v2 contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2Status {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

/// The six message kinds in Runtime-v2.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2MessageKind {
    StateRequest,
    StateResponse,
    ActionRequest,
    ActionResponse,
    ReconcileRequest,
    ReconcileResponse,
}

/// Release metadata required by every Runtime-v2 message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Metadata {
    pub protocol_version: String,
    pub schema_digest: String,
    pub provenance: RuntimeV2Provenance,
}

impl RuntimeV2Metadata {
    /// Creates metadata for the owner-local copied artifact.
    #[must_use]
    pub fn new() -> Self {
        Self {
            protocol_version: RUNTIME_V2_PROTOCOL_VERSION.to_owned(),
            schema_digest: RUNTIME_V2_SCHEMA_DIGEST.to_owned(),
            provenance: RuntimeV2Provenance::default(),
        }
    }

    /// Validates the fixed protocol version, digest, and provenance.
    pub fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.protocol_version != RUNTIME_V2_PROTOCOL_VERSION
            || self.schema_digest != RUNTIME_V2_SCHEMA_DIGEST
            || !is_digest(&self.schema_digest)
        {
            return Err(RuntimeV2ValidationError::Metadata);
        }
        self.provenance.validate()
    }
}

impl Default for RuntimeV2Metadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Provenance identifying the inert release-like Runtime-v2 artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Provenance {
    pub artifact: String,
    pub source: String,
    pub generator: String,
}

impl Default for RuntimeV2Provenance {
    fn default() -> Self {
        Self {
            artifact: RUNTIME_V2_ARTIFACT.to_owned(),
            source: RUNTIME_V2_SCHEMA_SOURCE.to_owned(),
            generator: RUNTIME_V2_GENERATOR.to_owned(),
        }
    }
}

impl RuntimeV2Provenance {
    fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.artifact != RUNTIME_V2_ARTIFACT
            || self.source != RUNTIME_V2_SCHEMA_SOURCE
            || self.generator != RUNTIME_V2_GENERATOR
        {
            return Err(RuntimeV2ValidationError::Provenance);
        }
        Ok(())
    }
}

/// Bounded domain state carried by state and operation receipts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Observation {
    pub combat_phase: RuntimeV2CombatPhase,
    pub turn_index: u16,
    pub host_ready: bool,
    pub generation: u64,
}

impl RuntimeV2Observation {
    /// Creates a bounded observation without asserting live host compatibility.
    #[must_use]
    pub const fn new(
        combat_phase: RuntimeV2CombatPhase,
        turn_index: u16,
        host_ready: bool,
        generation: u64,
    ) -> Self {
        Self {
            combat_phase,
            turn_index,
            host_ready,
            generation,
        }
    }

    /// Validates the turn-index and generation bounds.
    pub fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.turn_index > RUNTIME_V2_MAX_TURN_INDEX {
            return Err(RuntimeV2ValidationError::ObservationBounds);
        }
        if self.generation > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2ValidationError::GenerationBounds);
        }
        Ok(())
    }
}

/// The fixed action. It intentionally contains no arguments.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Action {
    pub action_id: String,
}

impl RuntimeV2Action {
    /// Creates the only action admitted by this profile.
    #[must_use]
    pub fn end_turn() -> Self {
        Self {
            action_id: RUNTIME_V2_ACTION_ID.to_owned(),
        }
    }

    /// Creates an action value for validation tests or a boundary decoder.
    #[must_use]
    pub fn new(action_id: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
        }
    }

    /// Validates the fixed action identity.
    pub fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.action_id != RUNTIME_V2_ACTION_ID {
            return Err(RuntimeV2ValidationError::ActionBounds);
        }
        Ok(())
    }
}

/// The authoritative witness required for a settled end-turn effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2EffectWitness {
    pub kind: String,
    pub generation: u64,
}

impl RuntimeV2EffectWitness {
    /// Creates the fixed settlement witness.
    #[must_use]
    pub fn turn_end_settled(generation: u64) -> Self {
        Self {
            kind: RUNTIME_V2_EFFECT_KIND.to_owned(),
            generation,
        }
    }

    /// Validates the witness identity and generation bound.
    pub fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.kind != RUNTIME_V2_EFFECT_KIND {
            return Err(RuntimeV2ValidationError::EffectBounds);
        }
        if self.generation > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2ValidationError::GenerationBounds);
        }
        Ok(())
    }
}

/// A complete Runtime-v2 request, response, or retained receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Message {
    pub protocol_version: String,
    pub schema_digest: String,
    pub provenance: RuntimeV2Provenance,
    pub correlation_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub generation: u64,
    pub kind: RuntimeV2MessageKind,
    #[serde(deserialize_with = "required_nullable")]
    pub operation_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub observation: Option<RuntimeV2Observation>,
    #[serde(deserialize_with = "required_nullable")]
    pub action: Option<RuntimeV2Action>,
    #[serde(deserialize_with = "required_nullable")]
    pub status: Option<RuntimeV2Status>,
    #[serde(deserialize_with = "required_nullable")]
    pub error_code: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub effect_witness: Option<RuntimeV2EffectWitness>,
}

// A missing nullable member is not equivalent to an explicit null in the frozen wire contract.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
