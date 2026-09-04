// SPDX-License-Identifier: MIT

impl RuntimeV2Message {

    /// Validates metadata, bounds, identities, and kind-specific result shape.
    pub fn validate(&self) -> Result<(), RuntimeV2ValidationError> {
        self.validate_metadata()?;
        self.validate_common_fields()?;
        match self.kind {
            RuntimeV2MessageKind::StateRequest => self.validate_state_request(),
            RuntimeV2MessageKind::StateResponse => self.validate_state_response(),
            RuntimeV2MessageKind::ActionRequest => self.validate_action_request(),
            RuntimeV2MessageKind::ReconcileRequest => self.validate_reconcile_request(),
            RuntimeV2MessageKind::ActionResponse | RuntimeV2MessageKind::ReconcileResponse => {
                self.validate_result()
            }
        }
    }

    /// Returns the stable operation key for an action or reconciliation request/result.
    #[must_use]
    pub fn operation_key(&self) -> Option<RuntimeV2OperationKey> {
        self.operation_id.as_ref().map(|operation_id| {
            RuntimeV2OperationKey::new(
                &self.instance_id,
                &self.session_id,
                &self.lease_id,
                self.lease_epoch,
                operation_id,
            )
        })
    }

    /// Serializes the message with deterministic struct-field ordering.
    pub fn canonical_json(&self) -> Result<Vec<u8>, RuntimeV2CodecError> {
        serde_json::to_vec(self).map_err(|_| RuntimeV2CodecError::Serialization)
    }

    /// Serializes the request identity used for duplicate/conflict checks.
    ///
    /// Correlation is a per-transport-attempt identity, so a retry with a new MCP request ID must
    /// still replay the retained operation. The operation context, action, and generation remain
    /// part of this canonical identity and therefore still detect conflicting reuse.
    pub fn idempotency_canonical_json(&self) -> Result<Vec<u8>, RuntimeV2CodecError> {
        let mut identity = self.clone();
        identity.correlation_id.clear();
        serde_json::to_vec(&identity).map_err(|_| RuntimeV2CodecError::Serialization)
    }

    /// Computes the stable request digest used for duplicate/conflict checks.
    pub fn request_digest(&self) -> Result<RuntimeV2RequestDigest, RuntimeV2CodecError> {
        let bytes = self.idempotency_canonical_json()?;
        Ok(RuntimeV2RequestDigest(fnv1a(&bytes)))
    }

    #[allow(clippy::too_many_arguments)]
    fn base(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
        kind: RuntimeV2MessageKind,
    ) -> Self {
        Self {
            protocol_version: metadata.protocol_version,
            schema_digest: metadata.schema_digest,
            provenance: metadata.provenance,
            correlation_id: correlation_id.to_owned(),
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
            generation,
            kind,
            operation_id: None,
            observation: None,
            action: None,
            status: None,
            error_code: None,
            effect_witness: None,
        }
    }

    fn validate_metadata(&self) -> Result<(), RuntimeV2ValidationError> {
        RuntimeV2Metadata {
            protocol_version: self.protocol_version.clone(),
            schema_digest: self.schema_digest.clone(),
            provenance: self.provenance.clone(),
        }
        .validate()
    }

    fn validate_common_fields(&self) -> Result<(), RuntimeV2ValidationError> {
        for identity in [
            &self.correlation_id,
            &self.instance_id,
            &self.session_id,
            &self.lease_id,
        ] {
            validate_identity(identity)?;
        }
        if self.lease_epoch > RUNTIME_V2_MAX_GENERATION
            || self.generation > RUNTIME_V2_MAX_GENERATION
        {
            return Err(RuntimeV2ValidationError::GenerationBounds);
        }
        if let Some(operation_id) = &self.operation_id {
            validate_identity(operation_id)?;
        }
        if let Some(observation) = self.observation {
            observation.validate()?;
        }
        if let Some(action) = &self.action {
            action.validate()?;
        }
        if let Some(error_code) = &self.error_code {
            validate_identity(error_code)?;
        }
        if let Some(effect_witness) = self.effect_witness.as_ref() {
            effect_witness.validate()?;
        }
        Ok(())
    }

    fn validate_state_request(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.operation_id.is_none()
            && self.observation.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2ValidationError::ResultShape)
        }
    }

    fn validate_state_response(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.operation_id.is_none()
            && self.observation_generation_matches()
            && self.action.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2ValidationError::ResultShape)
        }
    }

    fn validate_action_request(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.operation_id.is_some()
            && self.action.is_some()
            && self.observation.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2ValidationError::ResultShape)
        }
    }

    fn validate_reconcile_request(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.operation_id.is_some()
            && self.action.is_none()
            && self.observation.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2ValidationError::ResultShape)
        }
    }

    fn validate_result(&self) -> Result<(), RuntimeV2ValidationError> {
        if self.operation_id.is_some()
            && self.action.is_some()
            && self.status.is_some()
            && self.result_fields_match_status()
        {
            Ok(())
        } else {
            Err(RuntimeV2ValidationError::ResultShape)
        }
    }

    fn result_fields_match_status(&self) -> bool {
        match self.status {
            Some(RuntimeV2Status::Accepted) => {
                self.observation_generation_matches()
                    && self.error_code.is_none()
                    && self.effect_witness.is_none()
            }
            Some(RuntimeV2Status::Settled) => {
                self.observation_generation_matches()
                    && self.error_code.is_none()
                    && self
                        .effect_witness
                        .as_ref()
                        .is_some_and(|witness| witness.generation == self.generation)
            }
            Some(RuntimeV2Status::Rejected | RuntimeV2Status::Cancelled) => {
                self.observation_generation_matches()
                    && self.error_code.is_some()
                    && self.effect_witness.is_none()
            }
            Some(RuntimeV2Status::Unknown) => {
                self.observation.is_none()
                    && self.error_code.is_some()
                    && self.effect_witness.is_none()
            }
            None => false,
        }
    }

    fn observation_generation_matches(&self) -> bool {
        self.observation
            .is_some_and(|observation| observation.generation == self.generation)
    }
}
