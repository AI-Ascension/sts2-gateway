// SPDX-License-Identifier: MIT

use serde_json::{Map, Value, json};
use sts2_gateway::RecoveryContinuationOwner;
use uuid::{Uuid, Variant};

pub(super) const CONTRACT: &str = "sts2-continuation-owner-v1";
pub(super) const SCHEMA_DIGEST: &str =
    "5e787126c98cf950b94dcb4e02c5520ebbc5e0571a5e827cf0bd286815cd49dc";
pub(super) const MAX_FRAME_BYTES: usize = 16 * 1024;
const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContinuationOwnerKind {
    Read,
    Claim,
    Lookup,
}

impl ContinuationOwnerKind {
    pub(super) const fn capability(self) -> &'static str {
        match self {
            Self::Read => "continuation_owner_read",
            Self::Claim => "continuation_owner_claim",
            Self::Lookup => "continuation_owner_lookup",
        }
    }

    const fn request_name(self) -> &'static str {
        match self {
            Self::Read => "current_owner_request",
            Self::Claim => "owner_claim_request",
            Self::Lookup => "owner_claim_lookup_request",
        }
    }

    pub(super) const fn response_name(self) -> &'static str {
        match self {
            Self::Read => "current_owner_response",
            Self::Claim => "owner_claim_response",
            Self::Lookup => "owner_claim_lookup_response",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ContinuationOwnerFrameError {
    Invalid,
    Oversized,
}

#[derive(Clone, Debug)]
pub(super) struct ContinuationOwnerFrame {
    value: Value,
}

impl ContinuationOwnerFrame {
    pub(super) fn parse(
        bytes: &[u8],
        kind: ContinuationOwnerKind,
    ) -> Result<Self, ContinuationOwnerFrameError> {
        if bytes.is_empty() {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ContinuationOwnerFrameError::Oversized);
        }
        let value =
            super::strict_json::parse(bytes).map_err(|_| ContinuationOwnerFrameError::Invalid)?;
        let object = value
            .as_object()
            .ok_or(ContinuationOwnerFrameError::Invalid)?;
        exact_keys(
            object,
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
        if value["contract"].as_str() != Some(CONTRACT)
            || value["schema_digest"].as_str() != Some(SCHEMA_DIGEST)
            || value["kind"].as_str() != Some(kind.request_name())
            || !uuid_v4(&value["message_id"])
            || !uuid_v4(&value["correlation_id"])
        {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
        let principal = value["actor"]["principal_id"]
            .as_str()
            .ok_or(ContinuationOwnerFrameError::Invalid)?;
        let actor = value["actor"]
            .as_object()
            .ok_or(ContinuationOwnerFrameError::Invalid)?;
        exact_keys(actor, &["principal_id", "role"])?;
        if principal.is_empty() || value["actor"]["role"] != "harness" {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
        let auth = value["auth"]
            .as_object()
            .ok_or(ContinuationOwnerFrameError::Invalid)?;
        exact_keys(auth, &["principal_id", "capability", "proof"])?;
        if value["auth"]["principal_id"] != principal
            || value["auth"]["capability"].as_str() != Some(kind.capability())
            || !value["auth"]["proof"].is_null()
        {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
        validate_payload(kind, &value["payload"])?;
        Ok(Self { value })
    }

    pub(super) fn correlation(&self) -> &str {
        self.value["correlation_id"].as_str().unwrap_or_default()
    }

    pub(super) fn principal(&self) -> &str {
        self.value["actor"]["principal_id"]
            .as_str()
            .unwrap_or_default()
    }

    pub(super) fn payload(&self) -> &Value {
        &self.value["payload"]
    }
}

pub(super) fn response_frame(
    kind: ContinuationOwnerKind,
    correlation_id: &str,
    principal_id: &str,
    payload: Value,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "contract": CONTRACT,
        "schema_digest": SCHEMA_DIGEST,
        "message_id": Uuid::new_v4().to_string(),
        "correlation_id": correlation_id,
        "actor": { "principal_id": principal_id, "role": "gateway" },
        "auth": {
            "principal_id": principal_id,
            "capability": kind.capability(),
            "proof": Value::Null,
        },
        "kind": kind.response_name(),
        "payload": payload,
    }))
    .unwrap_or_else(|_| b"{}".to_vec())
}

pub(super) fn owner_value(owner: &RecoveryContinuationOwner) -> Value {
    serde_json::to_value(owner).unwrap_or(Value::Null)
}

fn validate_payload(
    kind: ContinuationOwnerKind,
    value: &Value,
) -> Result<(), ContinuationOwnerFrameError> {
    let object = value
        .as_object()
        .ok_or(ContinuationOwnerFrameError::Invalid)?;
    match kind {
        ContinuationOwnerKind::Read => exact_keys(object, &[]),
        ContinuationOwnerKind::Claim => {
            exact_keys(object, &["operation_id", "expected_owner"])?;
            if !uuid_v4(&value["operation_id"]) {
                return Err(ContinuationOwnerFrameError::Invalid);
            }
            validate_owner(&value["expected_owner"])
        }
        ContinuationOwnerKind::Lookup => {
            exact_keys(object, &["operation_id"])?;
            if !uuid_v4(&value["operation_id"]) {
                return Err(ContinuationOwnerFrameError::Invalid);
            }
            Ok(())
        }
    }
}

fn validate_owner(value: &Value) -> Result<(), ContinuationOwnerFrameError> {
    let owner = value
        .as_object()
        .ok_or(ContinuationOwnerFrameError::Invalid)?;
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
            return Err(ContinuationOwnerFrameError::Invalid);
        }
    }
    for field in [
        "instance_incarnation",
        "boot_id",
        "host_fence_id",
        "lease_id",
    ] {
        if !uuid_v4(&value[field]) {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
    }
    if value["session_id"]
        .as_str()
        .is_none_or(|session| session.is_empty() || session.len() > 512)
    {
        return Err(ContinuationOwnerFrameError::Invalid);
    }
    for field in [
        "authority_generation",
        "host_fence_generation",
        "lease_epoch",
        "lease_expires_at_millis",
    ] {
        if value[field]
            .as_u64()
            .is_none_or(|number| number > MAX_WIRE_INTEGER)
        {
            return Err(ContinuationOwnerFrameError::Invalid);
        }
    }
    Ok(())
}

fn exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), ContinuationOwnerFrameError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(ContinuationOwnerFrameError::Invalid);
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
