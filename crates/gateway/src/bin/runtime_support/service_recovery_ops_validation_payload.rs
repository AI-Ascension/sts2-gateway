// SPDX-License-Identifier: MIT

fn parse_operation_state(value: &str) -> Option<RecoveryOperationState> {

    Some(match value {
        "INTENT_RECORDED" => RecoveryOperationState::IntentRecorded,
        "MAY_HAVE_BEEN_DISPATCHED" => RecoveryOperationState::MayHaveBeenDispatched,
        "ACCEPTED" => RecoveryOperationState::Accepted,
        "SETTLED" => RecoveryOperationState::Settled,
        "REJECTED" => RecoveryOperationState::Rejected,
        "UNKNOWN" => RecoveryOperationState::Unknown,
        "RECONCILED" => RecoveryOperationState::Reconciled,
        _ => return None,
    })
}

fn validate_expected_boundary(value: &Value, operation: &RecoveryOperation) -> bool {
    let Some(object) =
        super::recovery_wire::exact_object(value, &["state_id", "generation", "catalog_digest"])
    else {
        return false;
    };
    object["state_id"].as_str() == Some(operation.expected_state_id.as_str())
        && object["generation"].as_u64() == Some(operation.expected_generation)
        && object["catalog_digest"].as_str() == Some(operation.catalog_digest.as_str())
}

fn validate_action(value: &Value, operation: &RecoveryOperation) -> bool {
    let Some(object) = super::recovery_wire::exact_object(
        value,
        &["schema_digest", "canonical_json_b64", "payload_digest"],
    ) else {
        return false;
    };
    if object["schema_digest"].as_str() != Some(operation.schema_digest.as_str())
        || object["payload_digest"].as_str() != Some(operation.payload_digest.as_str())
    {
        return false;
    }
    let Some(encoded) = object["canonical_json_b64"].as_str() else {
        return false;
    };
    let decoded = decode_base64(encoded);
    decoded
        .and_then(|bytes| canonicalize_recovery_action(&bytes).ok())
        .is_some_and(|canonical| canonical == operation.canonical_json)
}

fn decode_base64(value: &str) -> Option<Vec<u8>> {
    let padded = format!("{value}{}", "=".repeat((4 - value.len() % 4) % 4));
    base64::engine::general_purpose::STANDARD
        .decode(&padded)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(&padded)
                .ok()
        })
}
