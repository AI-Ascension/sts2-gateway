// SPDX-License-Identifier: MIT

//! Boundary validation for the digest-free public checkpoint reference.
//!
//! The gateway accepts only the shared `exact-checkpoint-reference-v1` envelope: a closed object
//! with a keyed handle, permitted boundary and assurance labels, and a restore flag. Unknown
//! members, privileged digests, and unsupported versions are rejected before any route acts, and an
//! assurance that contradicts the restore flag is refused rather than normalized.

use serde_json::Value;

/// Public reference schema identifier.
pub const REFERENCE_SCHEMA: &str = "ascension.exact_checkpoint_reference.v1";
/// Accepted reference version.
pub const REFERENCE_VERSION: &str = "exact-checkpoint-reference-v1";
/// Serialized prefix of a keyed public handle.
pub const HANDLE_PREFIX: &str = "ckpt-h1:";
/// Contract recorded in the conformance case.
pub const REFERENCE_CONTRACT: &str = "sts2.protocol/exact-checkpoint-reference-v1";
/// Schema digest supplied by the protocol release-like artifact.
pub const REFERENCE_SCHEMA_DIGEST: &str =
    "028e00d06f9f2b16cb9097f47aedd057e74046a7cb2ba97362978e18029f48ab";
/// Maximum accepted boundary-label length.
pub const MAX_BOUNDARY_LABEL_BYTES: usize = 256;

/// Assurance labels accepted on the public envelope.
pub const ASSURANCE_LABELS: [&str; 5] = [
    "public_observation_only",
    "capture_only",
    "restore_supported",
    "restore_verified",
    "continuation_certified",
];

const ALLOWED_MEMBERS: [&str; 8] = [
    "schema",
    "reference_version",
    "handle",
    "occurrence",
    "boundary_kind",
    "boundary_phase",
    "assurance",
    "restore_verified",
];

const MANIFEST: &str =
    include_str!("../../../protocol-artifact/exact-checkpoint-reference-v1/manifest.json");
const CHECKSUMS: &str =
    include_str!("../../../protocol-artifact/exact-checkpoint-reference-v1/SHA256SUMS");
const SCHEMA: &str =
    include_str!("../../../protocol-artifact/exact-checkpoint-reference-v1/schema.json");
const REFERENCE: &str =
    include_str!("../../../protocol-artifact/exact-checkpoint-reference-v1/golden/reference.json");

/// Rejection reasons for an inbound public reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceError {
    /// The value is not a JSON object.
    NotAnObject,
    /// A required member is absent.
    MissingMember,
    /// A member outside the closed envelope is present, including any digest.
    UnexpectedMember,
    /// The schema or reference version is not the supported one.
    UnsupportedVersion,
    /// The handle does not match the keyed-handle grammar.
    InvalidHandle,
    /// The occurrence identifier is empty or outside the accepted alphabet.
    InvalidOccurrence,
    /// A boundary label is empty or too long.
    InvalidBoundary,
    /// The assurance is not one of the declared labels.
    InvalidAssurance,
    /// The restore flag is absent, non-boolean, or contradicts the assurance.
    InvalidRestoreFlag,
    /// The copied artifact does not match its recorded identity.
    ArtifactMismatch,
}

impl std::fmt::Display for ReferenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotAnObject => "reference is not an object",
            Self::MissingMember => "reference lacks a required member",
            Self::UnexpectedMember => "reference carries an unexpected member",
            Self::UnsupportedVersion => "reference version is unsupported",
            Self::InvalidHandle => "reference handle is invalid",
            Self::InvalidOccurrence => "reference occurrence is invalid",
            Self::InvalidBoundary => "reference boundary label is invalid",
            Self::InvalidAssurance => "reference assurance is invalid",
            Self::InvalidRestoreFlag => "reference restore flag is invalid",
            Self::ArtifactMismatch => "copied reference artifact is invalid",
        })
    }
}

impl std::error::Error for ReferenceError {}

/// Verifies the copied artifact identity, schema, and checksum inventory.
pub fn verify_exact_checkpoint_reference_artifact() -> Result<(), ReferenceError> {
    let manifest: Value =
        serde_json::from_str(MANIFEST).map_err(|_| ReferenceError::ArtifactMismatch)?;
    if manifest["artifact"] != "sts2-protocol/exact-checkpoint-reference-v1"
        || manifest["protocol_version"] != REFERENCE_VERSION
        || manifest["schema_digest"] != REFERENCE_SCHEMA_DIGEST
        || manifest["provenance"]["license"] != "MIT"
    {
        return Err(ReferenceError::ArtifactMismatch);
    }
    if !CHECKSUMS.lines().all(|line| {
        line.split_once("  ").is_some_and(|(digest, path)| {
            digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !path.is_empty()
        })
    }) {
        return Err(ReferenceError::ArtifactMismatch);
    }
    let schema: Value =
        serde_json::from_str(SCHEMA).map_err(|_| ReferenceError::ArtifactMismatch)?;
    if schema["$id"] != "sts2-exact-checkpoint-reference-v1"
        || schema["additionalProperties"] != false
        || schema["properties"].as_object().map(serde_json::Map::len) != Some(ALLOWED_MEMBERS.len())
    {
        return Err(ReferenceError::ArtifactMismatch);
    }
    let golden: Value =
        serde_json::from_str(REFERENCE).map_err(|_| ReferenceError::ArtifactMismatch)?;
    validate_exact_checkpoint_reference(&golden).map_err(|_| ReferenceError::ArtifactMismatch)
}

/// Validates an inbound public reference against the closed envelope.
pub fn validate_exact_checkpoint_reference(value: &Value) -> Result<(), ReferenceError> {
    let object = value.as_object().ok_or(ReferenceError::NotAnObject)?;
    if object
        .keys()
        .any(|key| !ALLOWED_MEMBERS.contains(&key.as_str()))
    {
        return Err(ReferenceError::UnexpectedMember);
    }
    for member in ALLOWED_MEMBERS {
        if !object.contains_key(member) {
            return Err(ReferenceError::MissingMember);
        }
    }
    if object["schema"] != REFERENCE_SCHEMA || object["reference_version"] != REFERENCE_VERSION {
        return Err(ReferenceError::UnsupportedVersion);
    }
    let handle = object["handle"]
        .as_str()
        .ok_or(ReferenceError::InvalidHandle)?;
    if !valid_handle(handle) {
        return Err(ReferenceError::InvalidHandle);
    }
    let occurrence = object["occurrence"]
        .as_str()
        .ok_or(ReferenceError::InvalidOccurrence)?;
    if !valid_occurrence(occurrence) {
        return Err(ReferenceError::InvalidOccurrence);
    }
    for member in ["boundary_kind", "boundary_phase"] {
        let label = object[member]
            .as_str()
            .ok_or(ReferenceError::InvalidBoundary)?;
        if label.is_empty() || label.len() > MAX_BOUNDARY_LABEL_BYTES || label.contains('\0') {
            return Err(ReferenceError::InvalidBoundary);
        }
    }
    let assurance = object["assurance"]
        .as_str()
        .ok_or(ReferenceError::InvalidAssurance)?;
    if !ASSURANCE_LABELS.contains(&assurance) {
        return Err(ReferenceError::InvalidAssurance);
    }
    let restore_verified = object["restore_verified"]
        .as_bool()
        .ok_or(ReferenceError::InvalidRestoreFlag)?;
    let claims_restore = matches!(assurance, "restore_verified" | "continuation_certified");
    if restore_verified != claims_restore {
        return Err(ReferenceError::InvalidRestoreFlag);
    }
    Ok(())
}

fn valid_handle(handle: &str) -> bool {
    let Some(hex) = handle.strip_prefix(HANDLE_PREFIX) else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_occurrence(occurrence: &str) -> bool {
    !occurrence.is_empty()
        && occurrence.len() <= MAX_BOUNDARY_LABEL_BYTES
        && occurrence
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}
