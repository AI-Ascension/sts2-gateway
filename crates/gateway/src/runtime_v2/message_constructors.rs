// SPDX-License-Identifier: MIT

impl RuntimeV2Message {

    /// Creates a state request with no operation or result fields.
    #[must_use]
    pub fn state_request(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
    ) -> Self {
        Self::base(
            metadata,
            correlation_id,
            instance_id,
            session_id,
            lease_id,
            lease_epoch,
            generation,
            RuntimeV2MessageKind::StateRequest,
        )
    }

    /// Creates a state response carrying a bounded observation.
    #[must_use]
    pub fn state_response(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        observation: RuntimeV2Observation,
    ) -> Self {
        Self {
            observation: Some(observation),
            generation: observation.generation,
            ..Self::base(
                metadata,
                correlation_id,
                instance_id,
                session_id,
                lease_id,
                lease_epoch,
                observation.generation,
                RuntimeV2MessageKind::StateResponse,
            )
        }
    }

    /// Creates an action request with a stable operation identity.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn action_request(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
        operation_id: &str,
        action: RuntimeV2Action,
    ) -> Self {
        Self {
            operation_id: Some(operation_id.to_owned()),
            action: Some(action),
            ..Self::base(
                metadata,
                correlation_id,
                instance_id,
                session_id,
                lease_id,
                lease_epoch,
                generation,
                RuntimeV2MessageKind::ActionRequest,
            )
        }
    }

    /// Creates a reconciliation request with no mutation-bearing action.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_request(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
        operation_id: &str,
    ) -> Self {
        Self {
            operation_id: Some(operation_id.to_owned()),
            ..Self::base(
                metadata,
                correlation_id,
                instance_id,
                session_id,
                lease_id,
                lease_epoch,
                generation,
                RuntimeV2MessageKind::ReconcileRequest,
            )
        }
    }

    /// Creates an action or reconciliation result with explicit result fields.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn result(
        metadata: RuntimeV2Metadata,
        correlation_id: &str,
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        generation: u64,
        operation_id: &str,
        action: RuntimeV2Action,
        status: RuntimeV2Status,
        observation: Option<RuntimeV2Observation>,
        error_code: Option<String>,
        effect_witness: Option<RuntimeV2EffectWitness>,
        kind: RuntimeV2MessageKind,
    ) -> Self {
        Self {
            operation_id: Some(operation_id.to_owned()),
            observation,
            action: Some(action),
            status: Some(status),
            error_code,
            effect_witness,
            ..Self::base(
                metadata,
                correlation_id,
                instance_id,
                session_id,
                lease_id,
                lease_epoch,
                generation,
                kind,
            )
        }
    }
}
