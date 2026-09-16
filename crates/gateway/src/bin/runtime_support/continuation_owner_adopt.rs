// SPDX-License-Identifier: MIT

use serde_json::{Map, Value, json};
use sts2_gateway::RecoveryContinuationOwner;
use uuid::{Uuid, Variant};

pub(super) const CONTRACT: &str = "sts2-continuation-owner-adopt-v1";
pub(super) const SCHEMA_DIGEST: &str =
    "7240e2f5054e5f6639ad12fa1ed66d40992f1234e49b693b2aacc53451dec380";
pub(super) const MAX_FRAME_BYTES: usize = 16 * 1024;
const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum AdoptFrameError {
    Invalid,
    Oversized,
}

#[derive(Clone, Debug)]
pub(super) struct AdoptFrame {
    value: Value,
    owner: RecoveryContinuationOwner,
}

impl AdoptFrame {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, AdoptFrameError> {
        if bytes.is_empty() {
            return Err(AdoptFrameError::Invalid);
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(AdoptFrameError::Oversized);
        }
        let value = super::strict_json::parse(bytes).map_err(|_| AdoptFrameError::Invalid)?;
        let frame = value.as_object().ok_or(AdoptFrameError::Invalid)?;
        exact_keys(
            frame,
            &[
                "contract",
                "schema_digest",
                "message_id",
                "correlation_id",
                "actor",
                "auth",
                "kind",
                "payload",
            ],
        )?;
        if value["contract"] != CONTRACT
            || value["schema_digest"] != SCHEMA_DIGEST
            || value["kind"] != "owner_adopt_request"
            || !uuid_v4(&value["message_id"])
            || !uuid_v4(&value["correlation_id"])
        {
            return Err(AdoptFrameError::Invalid);
        }
        let actor = value["actor"].as_object().ok_or(AdoptFrameError::Invalid)?;
        exact_keys(actor, &["principal_id", "role"])?;
        let principal = value["actor"]["principal_id"]
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or(AdoptFrameError::Invalid)?;
        if value["actor"]["role"] != "harness" {
            return Err(AdoptFrameError::Invalid);
        }
        let auth = value["auth"].as_object().ok_or(AdoptFrameError::Invalid)?;
        exact_keys(auth, &["principal_id", "capability", "proof"])?;
        if value["auth"]["principal_id"] != principal
            || value["auth"]["capability"] != "continuation_owner_adopt"
            || !value["auth"]["proof"].is_null()
        {
            return Err(AdoptFrameError::Invalid);
        }
        let payload = value["payload"]
            .as_object()
            .ok_or(AdoptFrameError::Invalid)?;
        exact_keys(payload, &["operation_id", "expected_owner"])?;
        if !uuid_v4(&value["payload"]["operation_id"]) {
            return Err(AdoptFrameError::Invalid);
        }
        let owner = parse_owner(&value["payload"]["expected_owner"])?;
        Ok(Self { value, owner })
    }

    pub(super) fn principal(&self) -> &str {
        self.value["actor"]["principal_id"]
            .as_str()
            .unwrap_or_default()
    }

    pub(super) fn correlation(&self) -> &str {
        self.value["correlation_id"].as_str().unwrap_or_default()
    }

    pub(super) fn operation_id(&self) -> &str {
        self.value["payload"]["operation_id"]
            .as_str()
            .unwrap_or_default()
    }

    pub(super) fn owner(&self) -> &RecoveryContinuationOwner {
        &self.owner
    }
}

pub(super) fn response_frame(correlation_id: &str, principal_id: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation_id,
        "actor": {"principal_id": principal_id, "role": "gateway"},
        "auth": {
            "principal_id": principal_id,
            "capability": "continuation_owner_adopt",
            "proof": Value::Null
        },
        "kind": "owner_adopt_response",
        "payload": payload
    }))
    .unwrap_or_else(|_| b"{}".to_vec())
}

fn parse_owner(value: &Value) -> Result<RecoveryContinuationOwner, AdoptFrameError> {
    let owner = value.as_object().ok_or(AdoptFrameError::Invalid)?;
    exact_keys(
        owner,
        &[
            "deployment_id",
            "instance_id",
            "instance_incarnation",
            "boot_id",
            "authority_generation",
            "host_fence_id",
            "host_fence_generation",
            "lease_id",
            "lease_epoch",
            "session_id",
            "lease_expires_at_millis",
        ],
    )?;
    for field in ["deployment_id", "instance_id"] {
        if !uuid(&value[field]) {
            return Err(AdoptFrameError::Invalid);
        }
    }
    for field in [
        "instance_incarnation",
        "boot_id",
        "host_fence_id",
        "lease_id",
    ] {
        if !uuid_v4(&value[field]) {
            return Err(AdoptFrameError::Invalid);
        }
    }
    if value["session_id"]
        .as_str()
        .is_none_or(|value| value.is_empty() || value.len() > 512)
    {
        return Err(AdoptFrameError::Invalid);
    }
    for field in [
        "authority_generation",
        "host_fence_generation",
        "lease_epoch",
        "lease_expires_at_millis",
    ] {
        if value[field]
            .as_u64()
            .is_none_or(|value| value > MAX_WIRE_INTEGER)
        {
            return Err(AdoptFrameError::Invalid);
        }
    }
    serde_json::from_value(value.clone()).map_err(|_| AdoptFrameError::Invalid)
}

fn exact_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<(), AdoptFrameError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(AdoptFrameError::Invalid);
    }
    Ok(())
}

fn uuid_v4(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        Uuid::parse_str(value).ok().is_some_and(|id| {
            id.hyphenated().to_string() == value
                && id.get_variant() == Variant::RFC4122
                && id.get_version_num() == 4
        })
    })
}

fn uuid(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        Uuid::parse_str(value).ok().is_some_and(|id| {
            id.hyphenated().to_string() == value && id.get_variant() == Variant::RFC4122
        })
    })
}
