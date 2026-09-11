// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::super::http::ReadError;

pub(super) fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

pub(super) fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

pub(super) fn safe_operation_id(value: &str) -> bool {
    safe_identity(value) && !value.contains('/')
}

pub(super) fn json_bytes(value: &Value) -> Vec<u8> {
    match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) => b"{\"error_code\":\"serialization_failed\"}".to_vec(),
    }
}

pub(super) fn json_error(code: &str) -> Vec<u8> {
    json_bytes(&json!({ "error_code": code }))
}

pub(super) fn json_overload(code: &str) -> Vec<u8> {
    json_bytes(&json!({
        "error_code": code,
        "retryable": true,
        "retry_after_ms": 1000
    }))
}

pub(super) fn read_error_status(error: ReadError) -> u16 {
    match error {
        ReadError::Timeout => 504,
        ReadError::Malformed | ReadError::Oversized => 502,
        ReadError::Unavailable => 503,
    }
}
