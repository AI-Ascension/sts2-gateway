// SPDX-License-Identifier: MIT

/// Validation failures for the seeded-run representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeededRunValidationError {
    Metadata,
    Provenance,
    InvalidIdentity,
    InvalidSeed,
    SeedBounds,
    ContextDigest,
    InvalidContext,
    ContextBounds,
    ContextOrdering,
    InvalidCompatibility,
    ContextDigestMismatch,
    ContextSerialization,
    ContextRequired,
    GenerationBounds,
    EffectWitness,
    RequestShape,
    ResponseShape,
    SettlementEvidence,
}

impl std::fmt::Display for SeededRunValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Metadata => "seeded-run metadata is unsupported",
            Self::Provenance => "seeded-run provenance is unsupported",
            Self::InvalidIdentity => "seeded-run identity is empty, unsafe, or too long",
            Self::InvalidSeed => "seeded-run seed is empty or contains a control character",
            Self::SeedBounds => "seeded-run seed exceeds its byte bound",
            Self::ContextDigest => "seeded-run context digest is not SHA-256",
            Self::InvalidContext => "seeded-run context identity or text is invalid",
            Self::ContextBounds => "seeded-run context exceeds its bound",
            Self::ContextOrdering => "seeded-run modifiers are not sorted and unique or acts are duplicated",
            Self::InvalidCompatibility => "seeded-run compatibility identity is invalid",
            Self::ContextDigestMismatch => "seeded-run context digest does not match its fields",
            Self::ContextSerialization => "seeded-run context cannot be canonically serialized",
            Self::ContextRequired => "seeded-run training requires a selected native context",
            Self::GenerationBounds => "seeded-run generation is outside its bound",
            Self::EffectWitness => "seeded-run effect witness is unsupported",
            Self::RequestShape => "seeded-run request fields do not match its kind",
            Self::ResponseShape => "seeded-run response fields do not match its status",
            Self::SettlementEvidence => "seeded-run settlement lacks fresh host evidence",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for SeededRunValidationError {}

fn validate_identity(value: &str) -> Result<(), SeededRunValidationError> {
    if value.is_empty()
        || value.len() > SEEDED_RUN_MAX_IDENTITY_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return Err(SeededRunValidationError::InvalidIdentity);
    }
    Ok(())
}

fn validate_seed(value: &str) -> Result<(), SeededRunValidationError> {
    if value.is_empty() {
        return Err(SeededRunValidationError::InvalidSeed);
    }
    if value.len() > SEEDED_RUN_MAX_SEED_BYTES {
        return Err(SeededRunValidationError::SeedBounds);
    }
    if value.chars().any(char::is_control) {
        return Err(SeededRunValidationError::InvalidSeed);
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}


impl SeededRunMessage {
    /// Validates metadata, bounds, and kind-specific result shape.
    pub fn validate(&self) -> Result<(), SeededRunValidationError> {
        if self.protocol_version != SEEDED_RUN_PROTOCOL_VERSION
            || self.schema_digest != SEEDED_RUN_SCHEMA_DIGEST
            || !is_digest(&self.schema_digest)
        {
            return Err(SeededRunValidationError::Metadata);
        }
        self.provenance.validate()?;
        for identity in [
            &self.correlation_id,
            &self.instance_id,
            &self.session_id,
            &self.lease_id,
            &self.operation_id,
        ] {
            validate_identity(identity)?;
        }
        if self.lease_epoch > SEEDED_RUN_MAX_GENERATION
            || self.generation > SEEDED_RUN_MAX_GENERATION
        {
            return Err(SeededRunValidationError::GenerationBounds);
        }
        if let Some(seed) = &self.requested_seed {
            validate_seed(seed)?;
        }
        if let Some(seed) = &self.canonical_seed {
            validate_seed(seed)?;
        }
        if let Some(error_code) = &self.error_code {
            validate_identity(error_code)?;
        }
        if let Some(digest) = &self.context_digest
            && !is_digest(digest)
        {
            return Err(SeededRunValidationError::ContextDigest);
        }
        if let Some(selected_context) = &self.selected_context {
            selected_context.validate()?;
            if self.context_digest.as_deref() != Some(selected_context.context_digest.as_str()) {
                return Err(SeededRunValidationError::ContextDigestMismatch);
            }
        } else if self.context_digest.is_some() {
            return Err(SeededRunValidationError::ContextRequired);
        }
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        if let Some(witness) = &self.effect_witness {
            witness.validate()?;
        }
        match self.kind {
            SeededRunMessageKind::StartRequest => self.validate_start_request(),
            SeededRunMessageKind::ReconcileRequest => self.validate_reconcile_request(),
            SeededRunMessageKind::StartResponse | SeededRunMessageKind::ReconcileResponse => {
                self.validate_result()
            }
        }
    }

    fn base(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        kind: SeededRunMessageKind,
        operation_id: String,
        requested_seed: Option<String>,
        run_mode: Option<SeededRunMode>,
        context_digest: Option<String>,
    ) -> Self {
        Self {
            protocol_version: SEEDED_RUN_PROTOCOL_VERSION.to_owned(),
            schema_digest: SEEDED_RUN_SCHEMA_DIGEST.to_owned(),
            provenance: metadata,
            correlation_id: context.correlation_id,
            instance_id: context.instance_id,
            session_id: context.session_id,
            lease_id: context.lease_id,
            lease_epoch: context.lease_epoch,
            generation: context.generation,
            kind,
            operation_id,
            requested_seed,
            run_mode,
            selected_context: None,
            context_digest,
            status: None,
            canonical_seed: None,
            observation: None,
            effect_witness: None,
            error_code: None,
        }
    }

    fn validate_start_request(&self) -> Result<(), SeededRunValidationError> {
        if self.selected_context.is_none() {
            return Err(SeededRunValidationError::ContextRequired);
        }
        if self.requested_seed.is_some()
            && self.run_mode.is_some()
            && self.status.is_none()
            && self.canonical_seed.is_none()
            && self.observation.is_none()
            && self.effect_witness.is_none()
            && self.error_code.is_none()
        {
            Ok(())
        } else {
            Err(SeededRunValidationError::RequestShape)
        }
    }

    fn validate_reconcile_request(&self) -> Result<(), SeededRunValidationError> {
        if self.requested_seed.is_none()
            && self.run_mode.is_none()
            && self.selected_context.is_none()
            && self.context_digest.is_none()
            && self.status.is_none()
            && self.canonical_seed.is_none()
            && self.observation.is_none()
            && self.effect_witness.is_none()
            && self.error_code.is_none()
        {
            Ok(())
        } else {
            Err(SeededRunValidationError::RequestShape)
        }
    }

    fn validate_result(&self) -> Result<(), SeededRunValidationError> {
        if !matches!(
            self.kind,
            SeededRunMessageKind::StartResponse | SeededRunMessageKind::ReconcileResponse
        ) {
            return Err(SeededRunValidationError::ResponseShape);
        }
        if self.requested_seed.is_none() || self.run_mode.is_none() || self.status.is_none() {
            return Err(SeededRunValidationError::ResponseShape);
        }
        if self.selected_context.is_none() {
            return Err(SeededRunValidationError::ContextRequired);
        }
        let valid = match self.status {
            Some(SeededRunStatus::Accepted) => {
                self.canonical_seed.is_none()
                    && self.observation.is_none()
                    && self.effect_witness.is_none()
                    && self.error_code.is_none()
            }
            Some(SeededRunStatus::Settled) => {
                let Some(canonical_seed) = self.canonical_seed.as_ref() else {
                    return Err(SeededRunValidationError::SettlementEvidence);
                };
                let Some(observation) = self.observation.as_ref() else {
                    return Err(SeededRunValidationError::SettlementEvidence);
                };
                let Some(witness) = self.effect_witness.as_ref() else {
                    return Err(SeededRunValidationError::SettlementEvidence);
                };
                observation.run_started
                    && observation.host_ready
                    && observation.canonical_seed == *canonical_seed
                    && witness.kind == SEEDED_RUN_EFFECT_KIND
                    && witness.canonical_seed == *canonical_seed
                    && witness.generation == observation.generation
                    && observation.generation > self.generation
                    && self.observation_context_matches(observation)
                    && self.error_code.is_none()
            }
            Some(SeededRunStatus::Rejected | SeededRunStatus::Cancelled) => {
                self.canonical_seed.is_none()
                    && self.observation.is_none()
                    && self.effect_witness.is_none()
                    && self.error_code.is_some()
            }
            Some(SeededRunStatus::Unknown) => {
                self.canonical_seed.is_none()
                    && self.observation.is_none()
                    && self.effect_witness.is_none()
                    && self.error_code.is_some()
            }
            None => false,
        };
        if valid {
            Ok(())
        } else {
            Err(SeededRunValidationError::ResponseShape)
        }
    }

    fn observation_context_matches(&self, observation: &SeededRunObservation) -> bool {
        match &self.selected_context {
            Some(selected_context) => {
                observation.selected_context_digest.as_deref()
                    == Some(selected_context.context_digest.as_str())
            }
            None => observation.selected_context_digest.is_none(),
        }
    }
}
