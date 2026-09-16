// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::http::{MAX_BODY_BYTES, ReadError};
use super::super::super::strict_json;
use super::super::{json_error, safe_identity};
use super::{GameInformationRequestError, GameInformationResponseError};

pub(super) fn lookup_binding_request_is_closed(body: &[u8]) -> bool {
    if body.len() > MAX_BODY_BYTES {
        return false;
    }
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
            .is_some_and(|epoch| epoch <= sts2_gateway::MAX_WIRE_INTEGER)
        && ["project_id", "run_id", "episode_id", "agent_id"]
            .into_iter()
            .all(|name| {
                object
                    .get(name)
                    .and_then(Value::as_str)
                    .is_some_and(safe_identity)
            })
}

pub(super) fn lookup_binding_request_is_discovery(body: &[u8]) -> bool {
    strict_json::parse(body)
        .ok()
        .and_then(|value| {
            value
                .get("operation")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some("discovery")
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
