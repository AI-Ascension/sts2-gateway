// SPDX-License-Identifier: MIT

/// Version of the explicit-seed launch contract.
pub const SEEDED_RUN_PROTOCOL_VERSION: &str = "seeded-run-v1";
/// Release-like artifact identity carried by the seeded-run contract.
pub const SEEDED_RUN_ARTIFACT: &str = "sts2-protocol/seeded-run-v1";
/// Repository-relative source schema path.
pub const SEEDED_RUN_SCHEMA_SOURCE: &str = "schemas/seeded-run-v1.schema.json";
/// Generator recorded in the hand-authored artifact.
pub const SEEDED_RUN_GENERATOR: &str = "hand-authored";
/// SHA-256 digest of the canonical source schema. Updated with the checked-in schema artifact.
pub const SEEDED_RUN_SCHEMA_DIGEST: &str =
    "5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8";
/// Maximum encoded seed size. The host remains the authority for game-specific syntax.
pub const SEEDED_RUN_MAX_SEED_BYTES: usize = 64;
/// Maximum identity size shared with the existing runtime profiles.
pub const SEEDED_RUN_MAX_IDENTITY_BYTES: usize = 128;
/// Maximum identity size for a concrete selected context.
pub const SEEDED_RUN_MAX_CONTEXT_ID_BYTES: usize = 128;
/// Maximum text size for an act, modifier, or context policy.
pub const SEEDED_RUN_MAX_CONTEXT_TEXT_BYTES: usize = 128;
/// Maximum number of native modifiers carried by one context.
pub const SEEDED_RUN_MAX_MODIFIERS: usize = 32;
/// Maximum number of acts carried by one context.
pub const SEEDED_RUN_MAX_ACTS: usize = 8;
/// Maximum JSON-safe generation and lease epoch.
pub const SEEDED_RUN_MAX_GENERATION: u64 = 9_007_199_254_740_991;
/// The only settlement witness admitted by this profile.
pub const SEEDED_RUN_EFFECT_KIND: &str = "run_started";

/// Semantic mode for one explicit seeded launch.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunMode {
    /// Start a run using a caller-selected seed for training or evaluation.
    SeededTraining,
    /// Reproduce a previously known seed for review.
    SeededReplay,
    /// Exercise the launch seam without claiming a training or replay result.
    Diagnostic,
}

/// Message kinds in the seeded-run profile.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunMessageKind {
    StartRequest,
    StartResponse,
    ReconcileRequest,
    ReconcileResponse,
}

/// Lifecycle outcomes for one seeded-run operation.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeededRunStatus {
    /// The operation was admitted; no host launch is claimed.
    Accepted,
    /// The host returned a canonical seed and a fresh run-start witness.
    Settled,
    /// The operation was rejected without a launch.
    Rejected,
    /// Delivery or host execution remains uncertain and must be reconciled.
    Unknown,
    /// The operation was cancelled before mutation-bearing dispatch.
    Cancelled,
}

/// Fresh bounded observation returned after an authoritative run launch.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunObservation {
    /// True only after the host reports that the run has started.
    pub run_started: bool,
    /// Whether the host remains ready after the launch.
    pub host_ready: bool,
    /// Host generation associated with the observation.
    pub generation: u64,
    /// Canonical seed read back from the host.
    pub canonical_seed: String,
    /// Digest of the context the host selected, when the host can report it.
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub selected_context_digest: Option<String>,
    /// Host phase immediately before the launch operation.
    pub phase_before: String,
    /// Host phase observed after the launch operation.
    pub phase_after: String,
    /// Bounded host/game/mod/engine identity for the observation.
    pub compatibility_identity: String,
}

impl SeededRunObservation {
    /// Validates observation bounds and the host-owned canonical seed value.
    pub fn validate(&self) -> Result<(), SeededRunValidationError> {
        if self.generation > SEEDED_RUN_MAX_GENERATION {
            return Err(SeededRunValidationError::GenerationBounds);
        }
        validate_seed(&self.canonical_seed)?;
        if let Some(digest) = &self.selected_context_digest
            && !is_digest(digest)
        {
            return Err(SeededRunValidationError::ContextDigest);
        }
        validate_identity(&self.phase_before)?;
        validate_identity(&self.phase_after)?;
        validate_identity(&self.compatibility_identity)
    }
}

/// Effect witness that distinguishes a host-started run from request admission.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunEffectWitness {
    /// Exact witness identity.
    pub kind: String,
    /// Host generation that produced the witness.
    pub generation: u64,
    /// Canonical seed associated with the started run.
    pub canonical_seed: String,
}

impl SeededRunEffectWitness {
    /// Validates the fixed witness identity and its bounded seed.
    pub fn validate(&self) -> Result<(), SeededRunValidationError> {
        if self.kind != SEEDED_RUN_EFFECT_KIND {
            return Err(SeededRunValidationError::EffectWitness);
        }
        if self.generation > SEEDED_RUN_MAX_GENERATION {
            return Err(SeededRunValidationError::GenerationBounds);
        }
        validate_seed(&self.canonical_seed)
    }
}

/// Identity and lease context preserved by every message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SeededRunContext {
    pub correlation_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub generation: u64,
}

impl SeededRunContext {
    /// Creates a context without granting authority to any caller.
    #[must_use]
    pub fn new(
        correlation_id: impl Into<String>,
        instance_id: impl Into<String>,
        session_id: impl Into<String>,
        lease_id: impl Into<String>,
        lease_epoch: u64,
        generation: u64,
    ) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            instance_id: instance_id.into(),
            session_id: session_id.into(),
            lease_id: lease_id.into(),
            lease_epoch,
            generation,
        }
    }
}

/// Artifact metadata required by every seeded-run message.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunProvenance {
    pub artifact: String,
    pub source: String,
    pub generator: String,
}

impl Default for SeededRunProvenance {
    fn default() -> Self {
        Self {
            artifact: SEEDED_RUN_ARTIFACT.to_owned(),
            source: SEEDED_RUN_SCHEMA_SOURCE.to_owned(),
            generator: SEEDED_RUN_GENERATOR.to_owned(),
        }
    }
}

impl SeededRunProvenance {
    fn validate(&self) -> Result<(), SeededRunValidationError> {
        if self.artifact != SEEDED_RUN_ARTIFACT
            || self.source != SEEDED_RUN_SCHEMA_SOURCE
            || self.generator != SEEDED_RUN_GENERATOR
        {
            return Err(SeededRunValidationError::Provenance);
        }
        Ok(())
    }
}
