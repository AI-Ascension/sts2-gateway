// SPDX-License-Identifier: MIT

pub(super) fn parse_operation_ref(value: &Value) -> Option<(String, String, Value)> {
    let object = super::recovery_wire::exact_object(
        value,
        &["operation_id", "payload_digest", "original_context"],
    )?;
    let operation_id = object["operation_id"].as_str()?.to_owned();
    let payload_digest = object["payload_digest"].as_str()?.to_owned();
    (super::recovery_wire::valid_uuid_v4(&operation_id)
        && super::recovery_wire::valid_digest(&payload_digest)
        && context_shape(&object["original_context"]))
    .then_some((
        operation_id,
        payload_digest,
        object["original_context"].clone(),
    ))
}

pub(super) type ParsedOperationPayload = (
    String,
    String,
    Value,
    (String, u64, String),
    (String, Vec<u8>),
);

pub(super) fn parse_operation_payload(value: &Value) -> Option<ParsedOperationPayload> {
    let object = super::recovery_wire::exact_object(
        value,
        &[
            "operation_id",
            "payload_digest",
            "original_context",
            "expected_boundary",
            "action",
        ],
    )?;
    let operation_id = object["operation_id"].as_str()?.to_owned();
    let payload_digest = object["payload_digest"].as_str()?.to_owned();
    if !super::recovery_wire::valid_uuid_v4(&operation_id)
        || !super::recovery_wire::valid_digest(&payload_digest)
        || !context_shape(&object["original_context"])
    {
        return None;
    }
    let boundary = super::recovery_wire::exact_object(
        &object["expected_boundary"],
        &["state_id", "generation", "catalog_digest"],
    )?;
    let state_id = boundary["state_id"].as_str()?.to_owned();
    let generation = boundary["generation"].as_u64()?;
    let catalog_digest = boundary["catalog_digest"].as_str()?.to_owned();
    if !super::recovery_wire::valid_uuid(&state_id)
        || !super::recovery_wire::valid_digest(&catalog_digest)
    {
        return None;
    }
    let action = super::recovery_wire::exact_object(
        &object["action"],
        &["schema_digest", "canonical_json_b64", "payload_digest"],
    )?;
    let schema_digest = action["schema_digest"].as_str()?.to_owned();
    let encoded = action["canonical_json_b64"].as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(encoded)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded)
                .ok()
        })?;
    if schema_digest != RUNTIME_V3_SCHEMA_DIGEST
        || action["payload_digest"].as_str()? != payload_digest
    {
        return None;
    }
    let canonical = canonicalize_recovery_action(&bytes).ok()?;
    if canonical.len() > sts2_gateway::MAX_RECOVERY_ACTION_BYTES
        || sha256_hex(&canonical) != payload_digest
    {
        return None;
    }
    Some((
        operation_id,
        payload_digest,
        object["original_context"].clone(),
        (state_id, generation, catalog_digest),
        (schema_digest, canonical),
    ))
}

fn context_shape(value: &Value) -> bool {
    let Some(object) = super::recovery_wire::exact_object(
        value,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "lease_id",
            "lease_epoch",
        ],
    ) else {
        return false;
    };
    super::recovery_wire::valid_uuid(object["deployment_id"].as_str().unwrap_or_default())
        && super::recovery_wire::valid_uuid(object["instance_id"].as_str().unwrap_or_default())
        && super::recovery_wire::valid_uuid_v4(
            object["instance_incarnation"].as_str().unwrap_or_default(),
        )
        && super::recovery_wire::valid_uuid_v4(object["boot_id"].as_str().unwrap_or_default())
        && super::recovery_wire::valid_uuid_v4(object["lease_id"].as_str().unwrap_or_default())
        && object["authority_generation"]
            .as_u64()
            .is_some_and(|value| value > 0)
        && object["lease_epoch"]
            .as_u64()
            .is_some_and(|value| value > 0)
}

pub(super) fn context_matches_proof(value: &Value, proof: &RecoveryLeaseProof) -> bool {
    value["deployment_id"].as_str() == Some(proof.deployment_id.as_str())
        && value["instance_id"].as_str() == Some(proof.instance_id.as_str())
        && value["instance_incarnation"].as_str() == Some(proof.instance_incarnation.as_str())
        && value["boot_id"].as_str() == Some(proof.boot_id.as_str())
        && value["authority_generation"].as_u64() == Some(proof.authority_generation)
        && value["lease_id"].as_str() == Some(proof.lease_id.as_str())
        && value["lease_epoch"].as_u64() == Some(proof.lease_epoch)
}

pub(super) fn operation_context_matches(
    operation: &RecoveryOperation,
    proof: &RecoveryLeaseProof,
) -> bool {
    operation.deployment_id == proof.deployment_id
        && operation.instance_id == proof.instance_id
        && operation.instance_incarnation == proof.instance_incarnation
        && operation.boot_id == proof.boot_id
        && operation.authority_generation == proof.authority_generation
        && operation.lease_id == proof.lease_id
        && operation.lease_epoch == proof.lease_epoch
}
