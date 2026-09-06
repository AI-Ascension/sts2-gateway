// SPDX-License-Identifier: MIT

use super::super::recovery_canonical::canonicalize_recovery_action;
use super::super::recovery_types::{
    RecoveryEffectWitness, RecoveryOperation, RecoveryOperationIntent, RecoveryOperationState,
    RecoveryStoreError, validate_digest, validate_identity, validate_uuid, validate_uuid_v4,
    validate_wire,
};
use super::super::{MAX_RECOVERY_ACTION_BYTES, sha256_hex};

pub(super) use super::operation_persistence::{
    insert_operation, select_operation, update_operation,
};

pub(super) fn validate_intent(intent: &RecoveryOperationIntent) -> Result<(), RecoveryStoreError> {
    for (name, value) in [
        ("operation_id", intent.operation_id.as_str()),
        ("deployment_id", intent.deployment_id.as_str()),
        ("instance_id", intent.instance_id.as_str()),
        ("instance_incarnation", intent.instance_incarnation.as_str()),
        ("boot_id", intent.boot_id.as_str()),
        ("lease_id", intent.lease_id.as_str()),
        ("expected_state_id", intent.expected_state_id.as_str()),
    ] {
        validate_identity(name, value)?;
    }
    validate_uuid_v4("operation_id", &intent.operation_id)?;
    validate_uuid("deployment_id", &intent.deployment_id)?;
    validate_uuid("instance_id", &intent.instance_id)?;
    validate_uuid_v4("instance_incarnation", &intent.instance_incarnation)?;
    validate_uuid_v4("boot_id", &intent.boot_id)?;
    validate_uuid_v4("lease_id", &intent.lease_id)?;
    validate_uuid("expected_state_id", &intent.expected_state_id)?;
    validate_digest("schema_digest", &intent.schema_digest)?;
    validate_digest("catalog_digest", &intent.catalog_digest)?;
    validate_digest("payload_digest", &intent.payload_digest)?;
    validate_wire(intent.authority_generation, "authority_generation")?;
    validate_wire(intent.lease_epoch, "lease_epoch")?;
    validate_wire(intent.expected_generation, "expected_generation")?;
    validate_wire(intent.now_millis, "now_millis")?;
    if intent.schema_digest != super::super::recovery_types::RUNTIME_V3_SCHEMA_DIGEST {
        return Err(RecoveryStoreError::ContractMismatch(
            "action schema digest is not the approved runtime-v3 profile".to_owned(),
        ));
    }
    if intent.canonical_json.is_empty() || intent.canonical_json.len() > MAX_RECOVERY_ACTION_BYTES {
        return Err(RecoveryStoreError::InvalidInput(
            "canonical action exceeds the recovery bound".to_owned(),
        ));
    }
    let canonical = canonicalize_recovery_action(&intent.canonical_json)?;
    if canonical != intent.canonical_json {
        return Err(RecoveryStoreError::ContractMismatch(
            "canonical action bytes do not match RCJ-1".to_owned(),
        ));
    }
    if sha256_hex(&canonical) != intent.payload_digest {
        return Err(RecoveryStoreError::ContractMismatch(
            "payload digest does not match canonical action bytes".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn same_intent(operation: &RecoveryOperation, intent: &RecoveryOperationIntent) -> bool {
    operation.payload_digest == intent.payload_digest
        && operation.schema_digest == intent.schema_digest
        && operation.canonical_json == intent.canonical_json
        && operation.deployment_id == intent.deployment_id
        && operation.instance_incarnation == intent.instance_incarnation
        && operation.boot_id == intent.boot_id
        && operation.authority_generation == intent.authority_generation
        && operation.lease_id == intent.lease_id
        && operation.lease_epoch == intent.lease_epoch
        && operation.expected_state_id == intent.expected_state_id
        && operation.expected_generation == intent.expected_generation
        && operation.catalog_digest == intent.catalog_digest
}

pub(super) fn valid_transition(from: RecoveryOperationState, to: RecoveryOperationState) -> bool {
    matches!(
        (from, to),
        (
            RecoveryOperationState::IntentRecorded,
            RecoveryOperationState::MayHaveBeenDispatched
        ) | (
            RecoveryOperationState::IntentRecorded,
            RecoveryOperationState::Rejected
        ) | (
            RecoveryOperationState::MayHaveBeenDispatched,
            RecoveryOperationState::Accepted
        ) | (
            RecoveryOperationState::MayHaveBeenDispatched,
            RecoveryOperationState::Settled
        ) | (
            RecoveryOperationState::MayHaveBeenDispatched,
            RecoveryOperationState::Rejected
        ) | (
            RecoveryOperationState::MayHaveBeenDispatched,
            RecoveryOperationState::Unknown
        ) | (
            RecoveryOperationState::Accepted,
            RecoveryOperationState::Settled
        ) | (
            RecoveryOperationState::Accepted,
            RecoveryOperationState::Unknown
        ) | (
            RecoveryOperationState::Unknown,
            RecoveryOperationState::Reconciled
        ) | (
            RecoveryOperationState::Accepted,
            RecoveryOperationState::Reconciled
        ) | (
            RecoveryOperationState::MayHaveBeenDispatched,
            RecoveryOperationState::Reconciled
        ) | (
            RecoveryOperationState::IntentRecorded,
            RecoveryOperationState::Unknown
        )
    )
}

pub(super) fn validate_witness_shape(
    operation_id: &str,
    state: RecoveryOperationState,
    witness: Option<&RecoveryEffectWitness>,
) -> Result<(), RecoveryStoreError> {
    if state == RecoveryOperationState::Settled && witness.is_none() {
        return Err(RecoveryStoreError::ContractMismatch(
            "settled operation requires an operation-specific witness".to_owned(),
        ));
    }
    if let Some(witness) = witness {
        if witness.operation_id != operation_id {
            return Err(RecoveryStoreError::ContractMismatch(
                "effect witness is bound to another operation".to_owned(),
            ));
        }
        validate_uuid_v4("witness_id", &witness.witness_id)?;
        validate_uuid_v4("operation_id", &witness.operation_id)?;
        validate_digest("witness_payload_digest", &witness.payload_digest)?;
        validate_uuid_v4("witness_boot_id", &witness.boot_id)?;
        validate_uuid_v4(
            "witness_instance_incarnation",
            &witness.instance_incarnation,
        )?;
        validate_uuid_v4("witness_host_fence_id", &witness.host_fence_id)?;
        validate_identity("witness_source", &witness.source)?;
        validate_uuid("witness_state_id", &witness.state_id)?;
        validate_digest("effect_digest", &witness.effect_digest)?;
        validate_wire(witness.generation, "witness_generation")?;
        validate_wire(witness.observed_at_millis, "witness_observed_at_millis")?;
        if !matches!(
            witness.source.as_str(),
            "host_game_thread" | "host_receipt" | "authoritative_reobserve"
        ) {
            return Err(RecoveryStoreError::ContractMismatch(
                "effect witness source is not authoritative".to_owned(),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_witness_context(
    operation: &RecoveryOperation,
    witness: Option<&RecoveryEffectWitness>,
) -> Result<(), RecoveryStoreError> {
    let Some(witness) = witness else {
        return Ok(());
    };
    if witness.payload_digest != operation.payload_digest
        || witness.boot_id != operation.boot_id
        || witness.instance_incarnation != operation.instance_incarnation
    {
        return Err(RecoveryStoreError::ContractMismatch(
            "effect witness does not match the operation authority context".to_owned(),
        ));
    }
    Ok(())
}
