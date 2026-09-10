// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use uuid::Uuid;

use sts2_gateway::{MAX_RECOVERY_FRAME_BYTES, RECOVERY_CONTRACT, RECOVERY_SCHEMA_DIGEST};

#[path = "recovery_frame_validation.rs"]
mod validation;

use validation::{actor, auth, exact_keys, validate_payload};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryKind {
    Bootstrap,
    HostFence,
    LeaseAcquire,
    LeaseRenew,
    LeaseRevoke,
    OperationIntent,
    OperationDispatch,
    OperationLookup,
    OperationReconcile,
}

impl RecoveryKind {
    pub(super) const fn request_name(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap_request",
            Self::HostFence => "host_fence_request",
            Self::LeaseAcquire => "lease_acquire_request",
            Self::LeaseRenew => "lease_renew_request",
            Self::LeaseRevoke => "lease_revoke_request",
            Self::OperationIntent => "operation_intent_request",
            Self::OperationDispatch => "operation_dispatch_request",
            Self::OperationLookup => "operation_lookup_request",
            Self::OperationReconcile => "operation_reconcile_request",
        }
    }

    pub(super) const fn response_name(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap_response",
            Self::HostFence => "host_fence_response",
            Self::LeaseAcquire => "lease_acquire_response",
            Self::LeaseRenew => "lease_renew_response",
            Self::LeaseRevoke => "lease_revoke_response",
            Self::OperationIntent => "operation_intent_response",
            Self::OperationDispatch => "operation_dispatch_response",
            Self::OperationLookup => "operation_lookup_response",
            Self::OperationReconcile => "operation_reconcile_response",
        }
    }

    pub(super) const fn capability(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::HostFence => "host_fence",
            Self::LeaseAcquire => "lease_acquire",
            Self::LeaseRenew => "lease_renew",
            Self::LeaseRevoke => "lease_revoke",
            Self::OperationIntent | Self::OperationDispatch => "operation_submit",
            Self::OperationLookup => "recovery_read",
            Self::OperationReconcile => "recovery_reconcile",
        }
    }
}

#[derive(Debug)]
pub(super) enum RecoveryFrameError {
    Invalid,
    Oversized,
}

#[derive(Clone, Debug)]
pub(super) struct RecoveryFrame {
    pub(super) value: Value,
}

pub(super) fn parse_response_frame(
    bytes: &[u8],
    kind: RecoveryKind,
) -> Result<Value, RecoveryFrameError> {
    if bytes.is_empty() {
        return Err(RecoveryFrameError::Invalid);
    }
    if bytes.len() > MAX_RECOVERY_FRAME_BYTES {
        return Err(RecoveryFrameError::Oversized);
    }
    let value = super::strict_json::parse(bytes).map_err(|_| RecoveryFrameError::Invalid)?;
    let object = value.as_object().ok_or(RecoveryFrameError::Invalid)?;
    exact_keys(
        object,
        &[
            "contract",
            "schema_digest",
            "message_id",
            "correlation_id",
            "sent_at",
            "actor",
            "auth",
            "kind",
            "payload",
        ],
    )?;
    if value["contract"].as_str() != Some(RECOVERY_CONTRACT)
        || value["schema_digest"].as_str() != Some(RECOVERY_SCHEMA_DIGEST)
        || value["kind"].as_str() != Some(kind.response_name())
        || !uuid_v4(&value["message_id"])
        || !uuid_v4(&value["correlation_id"])
        || !timestamp(&value["sent_at"])
    {
        return Err(RecoveryFrameError::Invalid);
    }
    actor(&value["actor"])?;
    auth(&value["auth"], kind.capability())?;
    if value["actor"]["principal_id"] != value["auth"]["principal_id"] {
        return Err(RecoveryFrameError::Invalid);
    }
    Ok(value)
}

impl RecoveryFrame {
    pub(super) fn parse(bytes: &[u8], kind: RecoveryKind) -> Result<Self, RecoveryFrameError> {
        if bytes.is_empty() {
            return Err(RecoveryFrameError::Invalid);
        }
        if bytes.len() > MAX_RECOVERY_FRAME_BYTES {
            return Err(RecoveryFrameError::Oversized);
        }
        let value = super::strict_json::parse(bytes).map_err(|_| RecoveryFrameError::Invalid)?;
        let object = value.as_object().ok_or(RecoveryFrameError::Invalid)?;
        exact_keys(
            object,
            &[
                "contract",
                "schema_digest",
                "message_id",
                "correlation_id",
                "sent_at",
                "actor",
                "auth",
                "kind",
                "payload",
            ],
        )?;
        if value["contract"].as_str() != Some(RECOVERY_CONTRACT)
            || value["schema_digest"].as_str() != Some(RECOVERY_SCHEMA_DIGEST)
            || value["kind"].as_str() != Some(kind.request_name())
            || !uuid_v4(&value["message_id"])
            || !uuid_v4(&value["correlation_id"])
            || !timestamp(&value["sent_at"])
        {
            return Err(RecoveryFrameError::Invalid);
        }
        actor(&value["actor"])?;
        auth(&value["auth"], kind.capability())?;
        if value["actor"]["principal_id"] != value["auth"]["principal_id"] {
            return Err(RecoveryFrameError::Invalid);
        }
        validate_payload(kind, &value["payload"])?;
        Ok(Self { value })
    }

    pub(super) fn correlation(&self) -> &str {
        self.value["correlation_id"].as_str().unwrap_or_default()
    }

    pub(super) fn payload(&self) -> &Value {
        &self.value["payload"]
    }

    pub(super) fn auth_proof(&self) -> Option<&str> {
        self.value["auth"]["proof"].as_str()
    }
}

/// Builds a closed recovery request for the gateway-to-host control channel. The caller supplies
/// the capability proof; this helper never derives one from gameplay response data.
pub(super) fn request_frame(
    kind: RecoveryKind,
    principal_id: &str,
    correlation: &str,
    proof: Option<&str>,
    payload: Value,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "contract": RECOVERY_CONTRACT,
        "schema_digest": RECOVERY_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation,
        "sent_at": now_timestamp(),
        "actor": { "principal_id": principal_id, "role": "gateway" },
        "auth": {
            "principal_id": principal_id,
            "capability": kind.capability(),
            "proof": proof,
        },
        "kind": kind.request_name(),
        "payload": payload,
    }))
    .unwrap_or_else(|_| b"{}".to_vec())
}

pub(super) fn response_frame(
    kind: RecoveryKind,
    correlation: &str,
    principal_id: &str,
    payload: Value,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "contract": RECOVERY_CONTRACT,
        "schema_digest": RECOVERY_SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation,
        "sent_at": now_timestamp(),
        "actor": { "principal_id": principal_id, "role": "gateway" },
        "auth": { "principal_id": principal_id, "capability": kind.capability(), "proof": Value::Null },
        "kind": kind.response_name(),
        "payload": payload,
    }))
    .unwrap_or_else(|_| b"{}".to_vec())
}

pub(super) fn response_result(
    status: &str,
    retryable: bool,
    retry_after_seconds: Option<u64>,
) -> Value {
    json!({
        "status": status,
        "retryable": retryable,
        "retry_after_seconds": retry_after_seconds,
    })
}

pub(super) fn now_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        });
    timestamp_from_millis(millis)
}

pub(super) fn timestamp_from_millis(millis: u64) -> String {
    let seconds = millis / 1_000;
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_seconds / 3_600,
        day_seconds / 60 % 60,
        day_seconds % 60,
        millis % 1_000
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}

/* moved */
fn uuid(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        Uuid::parse_str(text).is_ok()
            && Uuid::parse_str(text).ok().is_some_and(|id| {
                id.hyphenated().to_string() == text && id.get_variant() == uuid::Variant::RFC4122
            })
    })
}

fn uuid_v4(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        Uuid::parse_str(text).ok().is_some_and(|id| {
            id.hyphenated().to_string() == text
                && id.get_version_num() == 4
                && id.get_variant() == uuid::Variant::RFC4122
        })
    })
}

fn timestamp(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let bytes = text.as_bytes();
    (20..=30).contains(&bytes.len())
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes.last() == Some(&b'Z')
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7 | 10 | 13 | 16) || index == bytes.len() - 1 {
                true
            } else {
                byte.is_ascii_digit() || (*byte == b'.' && (17..bytes.len() - 1).contains(&index))
            }
        })
}
