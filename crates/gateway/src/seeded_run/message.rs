// SPDX-License-Identifier: MIT

/// Complete seeded-run request, response, or reconciliation receipt.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeededRunMessage {
    pub protocol_version: String,
    pub schema_digest: String,
    pub provenance: SeededRunProvenance,
    pub correlation_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub generation: u64,
    pub kind: SeededRunMessageKind,
    pub operation_id: String,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub requested_seed: Option<String>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub run_mode: Option<SeededRunMode>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub selected_context: Option<SeededRunSelectionContext>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub context_digest: Option<String>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub status: Option<SeededRunStatus>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub canonical_seed: Option<String>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub observation: Option<SeededRunObservation>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub effect_witness: Option<SeededRunEffectWitness>,
    #[serde(deserialize_with = "crate::seeded_run::required_option")]
    pub error_code: Option<String>,
}

impl SeededRunMessage {
    /// Creates an explicit-seed launch request.
    #[must_use]
    pub fn start_request(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        operation_id: impl Into<String>,
        requested_seed: impl Into<String>,
        run_mode: SeededRunMode,
        context_digest: Option<String>,
    ) -> Self {
        Self::base(
            metadata,
            context,
            SeededRunMessageKind::StartRequest,
            operation_id.into(),
            Some(requested_seed.into()),
            Some(run_mode),
            context_digest,
        )
    }

    /// Creates a launch request carrying a validated native context manifest.
    #[must_use]
    pub fn start_request_with_context(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        operation_id: impl Into<String>,
        requested_seed: impl Into<String>,
        run_mode: SeededRunMode,
        selected_context: SeededRunSelectionContext,
    ) -> Self {
        let digest = Some(selected_context.context_digest.clone());
        let mut message = Self::start_request(
            metadata,
            context,
            operation_id,
            requested_seed,
            run_mode,
            digest,
        );
        message.selected_context = Some(selected_context);
        message
    }

    /// Creates a read-only reconciliation request.
    #[must_use]
    pub fn reconcile_request(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        operation_id: impl Into<String>,
    ) -> Self {
        Self::base(
            metadata,
            context,
            SeededRunMessageKind::ReconcileRequest,
            operation_id.into(),
            None,
            None,
            None,
        )
    }

    /// Creates a lifecycle response or reconciliation receipt.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn result(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        kind: SeededRunMessageKind,
        operation_id: impl Into<String>,
        requested_seed: impl Into<String>,
        run_mode: SeededRunMode,
        context_digest: Option<String>,
        status: SeededRunStatus,
        canonical_seed: Option<String>,
        observation: Option<SeededRunObservation>,
        effect_witness: Option<SeededRunEffectWitness>,
        error_code: Option<String>,
    ) -> Self {
        let base = Self::base(
            metadata,
            context,
            kind,
            operation_id.into(),
            Some(requested_seed.into()),
            Some(run_mode),
            context_digest,
        );
        Self {
            status: Some(status),
            canonical_seed,
            observation,
            effect_witness,
            error_code,
            ..base
        }
    }

    /// Creates a lifecycle result carrying the selected native context.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn result_with_context(
        metadata: SeededRunProvenance,
        context: SeededRunContext,
        kind: SeededRunMessageKind,
        operation_id: impl Into<String>,
        requested_seed: impl Into<String>,
        run_mode: SeededRunMode,
        selected_context: SeededRunSelectionContext,
        status: SeededRunStatus,
        canonical_seed: Option<String>,
        observation: Option<SeededRunObservation>,
        effect_witness: Option<SeededRunEffectWitness>,
        error_code: Option<String>,
    ) -> Self {
        let digest = Some(selected_context.context_digest.clone());
        let mut message = Self::result(
            metadata,
            context,
            kind,
            operation_id,
            requested_seed,
            run_mode,
            digest,
            status,
            canonical_seed,
            observation,
            effect_witness,
            error_code,
        );
        message.selected_context = Some(selected_context);
        message
    }

}
