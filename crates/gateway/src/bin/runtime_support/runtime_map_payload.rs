// SPDX-License-Identifier: MIT

use serde_json::Value;

const MAX_TEXT_BYTES: usize = 128;
const MAX_REASON_BYTES: usize = 256;
const MAX_TIMEOUT_MILLIS: u64 = 120_000;

pub(super) fn validate(value: &Value) -> bool {
    let Some(snapshot) = value.get("snapshot").and_then(Value::as_object) else {
        return false;
    };
    snapshot
        .get("game_build")
        .is_some_and(|value| valid_text(value, MAX_TEXT_BYTES))
        && snapshot
            .get("mod_version")
            .is_some_and(|value| valid_text(value, MAX_TEXT_BYTES))
        && snapshot
            .get("reason")
            .is_some_and(|value| value.is_null() || valid_text(value, MAX_REASON_BYTES))
        && validate_timeout(value)
}

fn valid_text(value: &Value, maximum_bytes: usize) -> bool {
    value.as_str().is_some_and(|value| {
        !value.is_empty() && value.len() <= maximum_bytes && !value.chars().any(char::is_control)
    })
}

fn validate_timeout(value: &Value) -> bool {
    let Some(timeout) = value.get("timeout") else {
        return false;
    };
    let Some(timeout) = timeout.as_object() else {
        return timeout.is_null();
    };
    let (Some(timeout_millis), Some(elapsed_millis)) = (
        timeout.get("timeout_millis").and_then(Value::as_u64),
        timeout.get("elapsed_millis").and_then(Value::as_u64),
    ) else {
        return false;
    };
    (1..=MAX_TIMEOUT_MILLIS).contains(&timeout_millis)
        && elapsed_millis <= timeout_millis
        && elapsed_millis <= MAX_TIMEOUT_MILLIS
}
