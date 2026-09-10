// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::super::{MAX_BODY_BYTES, MAX_RESPONSE_BYTES};
#[path = "service_receipt_query_json.rs"]
mod canonical;
#[path = "service_receipt_query_semantics.rs"]
mod semantics;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/schema.json"
));
const PROTOCOL_VERSION: &str = "coop-receipt-query-v1";
const SCHEMA_DIGEST: &str = "3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c";
const ARTIFACT: &str = "sts2-protocol/coop-receipt-query-v1";
const SCHEMA_SOURCE: &str = "schemas/coop-receipt-query-v1.schema.json";
const GENERATOR: &str = "hand-authored";
const MAX_GENERATION: u64 = 9_007_199_254_740_991;
const MIN_PARTICIPANTS: usize = 2;
const MAX_PARTICIPANTS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReceiptQueryValidationError {
    RequestBodyRequired,
    RequestBodyOversized,
    RequestBodyInvalid,
    ResponseBodyOversized,
    ResponseBodyInvalid,
}
pub(crate) fn validate_request(
    body: &[u8],
    headers: &BTreeMap<String, String>,
) -> Result<Value, ReceiptQueryValidationError> {
    if body.is_empty() {
        return Err(ReceiptQueryValidationError::RequestBodyRequired);
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ReceiptQueryValidationError::RequestBodyOversized);
    }
    let value = parse_canonical(body).ok_or(ReceiptQueryValidationError::RequestBodyInvalid)?;
    if !base_valid(&value)
        || value["kind"] != "receipt_query_request"
        || !headers_match(&value, headers)
        || !request_semantics_valid(&value)
    {
        return Err(ReceiptQueryValidationError::RequestBodyInvalid);
    }
    Ok(value)
}

pub(crate) fn validate_response(
    request: &Value,
    status_code: u16,
    body: &[u8],
) -> Result<(), ReceiptQueryValidationError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(ReceiptQueryValidationError::ResponseBodyOversized);
    }
    let value = parse_canonical(body).ok_or(ReceiptQueryValidationError::ResponseBodyInvalid)?;
    if !base_valid(&value)
        || value["kind"] != "receipt_query_response"
        || value["evidence_scope"] != "retained_receipt"
        || !response_identity_matches(request, &value)
        || !semantics::response_semantics_valid(request, status_code, &value)
    {
        return Err(ReceiptQueryValidationError::ResponseBodyInvalid);
    }
    Ok(())
}

fn parse_canonical(body: &[u8]) -> Option<Value> {
    let value = super::super::super::strict_json::parse(body).ok()?;
    let canonical = canonical::canonical_envelope(&value)?;
    (canonical == body).then_some(value)
}

fn base_valid(value: &Value) -> bool {
    value["protocol_version"] == PROTOCOL_VERSION
        && value["schema_digest"] == SCHEMA_DIGEST
        && provenance_valid(&value["provenance"])
        && schema_valid(value)
        && base_identities_valid(value)
}

fn schema_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn provenance_valid(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == canonical::PROVENANCE_FIELDS.len()
        && value["artifact"] == ARTIFACT
        && value["source"] == SCHEMA_SOURCE
        && value["generator"] == GENERATOR
}

fn base_identities_valid(value: &Value) -> bool {
    [
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "run_id",
        "actor_id",
        "authority_id",
        "authority_epoch",
    ]
    .into_iter()
    .all(|field| {
        value[field]
            .as_str()
            .is_some_and(super::super::safe_identity)
    }) && value["operation_id"]
        .as_str()
        .is_some_and(super::super::safe_operation_id)
        && value["action_kind"].as_str().is_some_and(valid_action_kind)
        && value["action_fingerprint"]
            .as_str()
            .is_some_and(lower_hex_digest)
        && value["lease_epoch"]
            .as_u64()
            .is_some_and(|generation| generation <= MAX_GENERATION)
        && value["expected_host_generation"]
            .as_u64()
            .is_some_and(|generation| generation <= MAX_GENERATION)
        && value["before_host_generation"]
            .as_u64()
            .is_some_and(|generation| generation <= MAX_GENERATION)
        && location_valid(&value["location"])
        && participants_valid(&value["participant_ids"], value["actor_id"].as_str())
}

fn request_semantics_valid(value: &Value) -> bool {
    value["expected_host_generation"] == value["before_host_generation"]
        && value["status"].is_null()
        && value["evidence_scope"].is_null()
        && value["receipt"].is_null()
        && value["error_code"].is_null()
}

fn response_identity_matches(request: &Value, response: &Value) -> bool {
    [
        "protocol_version",
        "schema_digest",
        "provenance",
        "correlation_id",
        "instance_id",
        "session_id",
        "lease_id",
        "lease_epoch",
        "operation_id",
        "action_kind",
        "action_fingerprint",
        "run_id",
        "location",
        "actor_id",
        "authority_id",
        "authority_epoch",
        "expected_host_generation",
        "before_host_generation",
        "participant_ids",
    ]
    .into_iter()
    .all(|field| request[field] == response[field])
}

fn valid_action_kind(value: &str) -> bool {
    matches!(value, "end_turn" | "play_card")
}

fn lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn location_valid(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == canonical::LOCATION_FIELDS.len()
        && value["act_index"]
            .as_i64()
            .is_some_and(|value| value >= i32::MIN as i64 && value <= i32::MAX as i64)
        && optional_i32(&value["room_id"])
        && optional_coordinate(&value["coord"])
}

fn optional_i32(value: &Value) -> bool {
    value.is_null()
        || value
            .as_i64()
            .is_some_and(|value| value >= i32::MIN as i64 && value <= i32::MAX as i64)
}

fn optional_coordinate(value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == canonical::COORDINATE_FIELDS.len()
        && ["col", "row"].into_iter().all(|field| {
            value[field]
                .as_i64()
                .is_some_and(|value| value >= i32::MIN as i64 && value <= i32::MAX as i64)
        })
}

fn participants_valid(value: &Value, actor: Option<&str>) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if !(MIN_PARTICIPANTS..=MAX_PARTICIPANTS).contains(&items.len()) {
        return false;
    }
    let Some(actor) = actor else {
        return false;
    };
    let mut prior = None;
    for item in items {
        let Some(participant) = item.as_str() else {
            return false;
        };
        if !super::super::safe_identity(participant)
            || prior.is_some_and(|prior: &str| prior >= participant)
        {
            return false;
        }
        prior = Some(participant);
    }
    items.iter().any(|item| item.as_str() == Some(actor))
}

fn headers_match(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    [
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
        ("correlation_id", "x-sts2-correlation-id"),
    ]
    .into_iter()
    .all(|(field, header)| value[field].as_str() == headers.get(header).map(String::as_str))
        && value["lease_epoch"].as_u64()
            == headers
                .get("x-sts2-lease-epoch")
                .and_then(|value| value.parse::<u64>().ok())
}
