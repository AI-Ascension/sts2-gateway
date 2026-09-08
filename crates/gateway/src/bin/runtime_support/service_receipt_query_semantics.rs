// SPDX-License-Identifier: MIT

use serde_json::Value;

pub(super) fn response_semantics_valid(request: &Value, status_code: u16, value: &Value) -> bool {
    let Some(status) = value["status"].as_str() else {
        return false;
    };
    let http_status_valid = match status {
        "accepted" | "settled" => status_code == 200,
        "rejected" => status_code == 409,
        "unknown" => status_code == 503,
        "recovery_required" => matches!(status_code, 409 | 503),
        _ => false,
    };
    if !http_status_valid {
        return false;
    }

    match status {
        "accepted" => accepted_receipt_valid(value),
        "settled" => settled_receipt_valid(request, value),
        "rejected" => rejected_receipt_valid(value),
        "unknown" | "recovery_required" => {
            value["receipt"].is_null()
                && value["error_code"]
                    .as_str()
                    .is_some_and(super::super::super::safe_identity)
        }
        _ => false,
    }
}

fn accepted_receipt_valid(value: &Value) -> bool {
    value["error_code"].is_null()
        && value["receipt"].as_object().is_some_and(|receipt| {
            receipt.len() == super::canonical::RECEIPT_FIELDS.len()
                && receipt["status"] == "accepted"
                && receipt["after_host_generation"].is_null()
                && receipt["checkpoint_id"].is_null()
                && receipt["state_digest"].is_null()
                && receipt["effect_id"].is_null()
                && receipt["effect_kind"].is_null()
                && receipt["error_code"].is_null()
        })
}

fn settled_receipt_valid(request: &Value, value: &Value) -> bool {
    let Some(receipt) = value["receipt"].as_object() else {
        return false;
    };
    let Some(before) = request["before_host_generation"].as_u64() else {
        return false;
    };
    let Some(after) = receipt["after_host_generation"].as_u64() else {
        return false;
    };
    let Some(action_kind) = request["action_kind"].as_str() else {
        return false;
    };
    let Some(operation_id) = request["operation_id"].as_str() else {
        return false;
    };
    let expected_effect_id = format!("effect:{operation_id}");
    let expected_effect_kind = format!("{action_kind}_settled");
    receipt.len() == super::canonical::RECEIPT_FIELDS.len()
        && value["error_code"].is_null()
        && receipt["status"] == "settled"
        && after > before
        && after <= super::MAX_GENERATION
        && receipt["checkpoint_id"]
            .as_str()
            .is_some_and(super::super::super::safe_identity)
        && receipt["state_digest"]
            .as_str()
            .is_some_and(super::lower_hex_digest)
        && receipt["effect_id"].as_str() == Some(expected_effect_id.as_str())
        && receipt["effect_kind"].as_str() == Some(expected_effect_kind.as_str())
        && receipt["error_code"].is_null()
}

fn rejected_receipt_valid(value: &Value) -> bool {
    let Some(error_code) = value["error_code"].as_str() else {
        return false;
    };
    value["receipt"].as_object().is_some_and(|receipt| {
        receipt.len() == super::canonical::RECEIPT_FIELDS.len()
            && receipt["status"] == "rejected"
            && receipt["after_host_generation"].is_null()
            && receipt["checkpoint_id"].is_null()
            && receipt["state_digest"].is_null()
            && receipt["effect_id"].is_null()
            && receipt["effect_kind"].is_null()
            && receipt["error_code"].as_str() == Some(error_code)
    })
}
