// SPDX-License-Identifier: MIT

//! Pinned validation for the whole-manifest read.
//!
//! The owner emits one complete `game-information-content-manifest-v1` envelope.  The gateway is a
//! named consumer of that closed contract, so it pins the schema digest and provenance, refuses
//! any envelope that does not carry the caller's correlation identity, and never truncates.  A
//! manifest the producer declares larger than this route admits is answered with the protocol's
//! own closed oversize arm, because a shorter manifest would be a different, plausible catalog.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::http::MAX_RESPONSE_BYTES;
use super::strict_json;

pub(crate) const SCHEMA_DIGEST: &str =
    "416a39769445e6e462c5d5b5504f29010c255e2116a73094e55c7268e47f2ba6";
pub(crate) const PROFILE: &str = "game-information-content-manifest-v1";
pub(crate) const ARTIFACT: &str = "sts2-protocol/game-information-content-manifest-v1";
pub(crate) const SCHEMA_SOURCE: &str = "schemas/game-information-content-manifest-v1.schema.json";
pub(crate) const GENERATOR: &str = "hand-authored";

/// The bound this route admits for one complete manifest.
///
/// The pinned profile permits a 16 MiB *message*, which is the producer's own serialization
/// ceiling.  This route admits the gateway's smaller framing bound and advertises it to consumers
/// through ADR 0036 rather than pretending to carry a message it would have to shorten.
pub(crate) const MAX_MANIFEST_BYTES: usize = MAX_RESPONSE_BYTES;

/// The longest correlation identity the pinned schema's `id` token admits.
const MAX_CORRELATION_BYTES: usize = 256;
const OVERSIZED_CODE: &str = "result_limit_exceeded";
const OVERSIZED_REASON: &str = "serialized_payload_too_large";
const REVISION_DIGEST_BYTES: usize = 64;
pub(crate) const CORRELATION_HEADER: &str = "x-sts2-correlation-id";

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/game-information-content-manifest-v1/schema.json"
));

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Oversized,
    Invalid,
    Revision,
}

/// Whether a caller-supplied correlation identity is the token the pinned schema admits.
///
/// The refusal envelope quotes this value, so it is checked before any downstream exchange.
pub(crate) fn correlation_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CORRELATION_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'/' | b'_' | b'-')
        })
}

/// The canonical revision the deployment has already pinned, when it is a manifest digest.
///
/// Query-v1 names the canonical `inventory_revision` in `content_manifest_id`.  A deployment that
/// has produced the manifest configures that digest here, and this route then refuses a manifest
/// whose `inventory_revision` differs, so a consumer cannot read content the admitted query scope
/// would reject.  A label that is not a digest pins no revision and is not compared.
pub(crate) fn pinned_revision(configured: &str) -> Option<&str> {
    (configured.len() == REVISION_DIGEST_BYTES
        && configured
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(configured)
}

/// The protocol's closed oversize arm for a manifest this route admits no complete form of.
///
/// The caller must have established [`correlation_identity`] first: the identity is interpolated
/// unescaped, which is exactly the alphabet the pinned schema's `id` token permits.  It is not a
/// producer message and carries no manifest bytes.
pub(crate) fn refusal_envelope(correlation_id: &str) -> Vec<u8> {
    format!(
        concat!(
            r#"{{"protocol_version":"{profile}","schema_digest":"{digest}","provenance":"#,
            r#"{{"artifact":"{artifact}","source":"{source}","generator":"{generator}"}},"#,
            r#""correlation_id":"{correlation}","kind":"error_response","manifest":null,"#,
            r#""error":{{"code":"{code}","reason":"{reason}"}}}}"#
        ),
        profile = PROFILE,
        digest = SCHEMA_DIGEST,
        artifact = ARTIFACT,
        source = SCHEMA_SOURCE,
        generator = GENERATOR,
        correlation = correlation_id,
        code = OVERSIZED_CODE,
        reason = OVERSIZED_REASON,
    )
    .into_bytes()
}

/// Validates one complete producer exchange against the pinned profile and the admitted bound.
pub(crate) fn validate_response(
    body: &[u8],
    headers: &BTreeMap<String, String>,
    pinned: Option<&str>,
    status: u16,
) -> Result<(), Error> {
    if body.len() > MAX_MANIFEST_BYTES {
        return Err(Error::Oversized);
    }
    let value = strict_json::parse(body).map_err(|_| Error::Invalid)?;
    if !base_valid(&value) || !correlation_matches(&value, headers) {
        return Err(Error::Invalid);
    }
    match value.get("kind").and_then(Value::as_str) {
        Some("error_response") => {
            if !(400..=599).contains(&status) {
                return Err(Error::Invalid);
            }
            Ok(())
        }
        Some("content_manifest_response") => {
            if status != 200 {
                return Err(Error::Invalid);
            }
            if let Some(pinned) = pinned
                && value["manifest"]["inventory_revision"].as_str() != Some(pinned)
            {
                return Err(Error::Revision);
            }
            Ok(())
        }
        _ => Err(Error::Invalid),
    }
}

fn base_valid(value: &Value) -> bool {
    value.get("protocol_version").and_then(Value::as_str) == Some(PROFILE)
        && value.get("schema_digest").and_then(Value::as_str) == Some(SCHEMA_DIGEST)
        && provenance_valid(value.get("provenance"))
        && schema_valid(value)
}

/// The pinned schema is only trustworthy while the copied artifact still hashes to the pin.
fn schema_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            if sts2_gateway::sha256_hex(SCHEMA.as_bytes()) != SCHEMA_DIGEST {
                return None;
            }
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| validator.is_valid(value))
}

fn provenance_valid(value: Option<&Value>) -> bool {
    let Some(provenance) = value.and_then(Value::as_object) else {
        return false;
    };
    provenance.len() == 3
        && provenance.get("artifact").and_then(Value::as_str) == Some(ARTIFACT)
        && provenance.get("source").and_then(Value::as_str) == Some(SCHEMA_SOURCE)
        && provenance.get("generator").and_then(Value::as_str) == Some(GENERATOR)
}

fn correlation_matches(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    value.get("correlation_id").and_then(Value::as_str)
        == headers.get(CORRELATION_HEADER).map(String::as_str)
}
