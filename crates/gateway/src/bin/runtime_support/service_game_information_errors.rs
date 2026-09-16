// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::http::ReadError;
use super::super::super::strict_json;
use super::super::{json_error, safe_identity};
use super::{GameInformationRequestError, GameInformationResponseError};

pub(super) fn lookup_binding_request_is_closed(body: &[u8]) -> bool {
    let Ok(value) = strict_json::parse(body) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 6
        && object.keys().all(|name| {
            matches!(
                name.as_str(),
                "operation"
                    | "project_id"
                    | "run_id"
                    | "episode_id"
                    | "agent_id"
                    | "authority_epoch"
            )
        })
        && matches!(
            object.get("operation").and_then(Value::as_str),
            Some("discovery" | "observe")
        )
        && object
            .get("authority_epoch")
            .and_then(Value::as_u64)
            .is_some()
        && ["project_id", "run_id", "episode_id", "agent_id"]
            .into_iter()
            .all(|name| {
                object
                    .get(name)
                    .and_then(Value::as_str)
                    .is_some_and(safe_identity)
            })
}

pub(super) fn lookup_binding_response_is_closed(body: &[u8], correlation: &str) -> bool {
    const VERSION: &str = "game-information-lookup-binding-v1";
    const DIGEST: &str = "f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58";
    let Ok(value) = strict_json::parse(body) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let required = [
        "protocol_version",
        "schema_digest",
        "provenance",
        "correlation_id",
        "kind",
        "binding",
        "discovery",
        "observation",
        "error",
    ];
    if object.len() != required.len()
        || required.iter().any(|key| !object.contains_key(*key))
        || object.get("protocol_version").and_then(Value::as_str) != Some(VERSION)
        || object.get("schema_digest").and_then(Value::as_str) != Some(DIGEST)
        || object.get("correlation_id").and_then(Value::as_str) != Some(correlation)
    {
        return false;
    }
    let Some(p) = object.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    if p.len() != 3
        || p.get("artifact").and_then(Value::as_str)
            != Some("sts2-protocol/game-information-lookup-binding-v1")
        || p.get("source").and_then(Value::as_str)
            != Some("schemas/game-information-lookup-binding-v1.schema.json")
        || p.get("generator").and_then(Value::as_str) != Some("hand-authored")
    {
        return false;
    }
    match object.get("kind").and_then(Value::as_str) {
        Some("error_response") => {
            object.get("binding") == Some(&Value::Null)
                && object.get("discovery") == Some(&Value::Null)
                && object.get("observation") == Some(&Value::Null)
                && object
                    .get("error")
                    .and_then(Value::as_object)
                    .is_some_and(|v| v.len() == 3)
        }
        Some("lookup_binding_discovery_response") => {
            object.get("binding") != Some(&Value::Null)
                && object.get("discovery") != Some(&Value::Null)
                && object.get("observation") == Some(&Value::Null)
                && object.get("error") == Some(&Value::Null)
        }
        Some("lookup_binding_observation_response") => {
            object.get("binding") != Some(&Value::Null)
                && object.get("discovery") != Some(&Value::Null)
                && object.get("observation") != Some(&Value::Null)
                && object.get("error") == Some(&Value::Null)
        }
        _ => false,
    }
}

pub(super) fn game_information_request_error(error: GameInformationRequestError) -> (u16, Vec<u8>) {
    let code = match error {
        GameInformationRequestError::Required => (400, "game_information_body_required"),
        GameInformationRequestError::Oversized => (413, "game_information_request_oversized"),
        GameInformationRequestError::Invalid => (400, "game_information_request_invalid"),
        GameInformationRequestError::Scope => (409, "game_information_scope_rejected"),
        GameInformationRequestError::Limit => (413, "game_information_limits_rejected"),
    };
    (code.0, json_error(code.1))
}

pub(super) fn game_information_response_error(
    error: GameInformationResponseError,
) -> (u16, Vec<u8>) {
    let code = match error {
        GameInformationResponseError::Oversized => "game_information_response_oversized",
        GameInformationResponseError::Invalid => "game_information_response_invalid",
    };
    (502, json_error(code))
}

pub(super) fn game_information_transport_error(error: ReadError) -> (u16, Vec<u8>) {
    match error {
        ReadError::Cancelled => (499, json_error("game_information_cancelled")),
        ReadError::Oversized => {
            game_information_response_error(GameInformationResponseError::Oversized)
        }
        ReadError::Malformed => {
            game_information_response_error(GameInformationResponseError::Invalid)
        }
        ReadError::Timeout => (504, json_error("game_information_downstream_timeout")),
        ReadError::Unavailable => (503, json_error("game_information_downstream_unavailable")),
    }
}
