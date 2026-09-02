// SPDX-License-Identifier: MIT

use serde_json::Value;

/// Version consumed by the gateway POC routing proof.
pub const POC_PROTOCOL_VERSION: &str = "poc-v1";
/// Schema digest supplied by the protocol release-like artifact.
pub const POC_SCHEMA_DIGEST: &str =
    "adb434d119a51b00d968e71bf0bf774f2a08de7c875a5479900aa34b3c02e027";
/// Release-like artifact identity, not a Rust package dependency.
pub const POC_ARTIFACT: &str = "sts2-protocol/poc-v1";
/// Repository-relative source recorded in the artifact provenance.
pub const POC_SCHEMA_SOURCE: &str = "schemas/poc-v1.schema.json";
/// Generator recorded in the hand-authored artifact.
pub const POC_GENERATOR: &str = "hand-authored";
/// Maximum fake budget represented by the bounded contract.
pub const POC_MAX_UNITS: u16 = 8;
/// Maximum settled-effect count represented by the bounded contract.
pub const POC_MAX_SETTLED_EFFECTS: u16 = 4;

const MANIFEST: &str = include_str!("../../../protocol-artifact/poc-v1/manifest.json");
const SOURCE_SCHEMA: &str = include_str!("../../../schemas/poc-v1.schema.json");
const SCHEMA: &str = include_str!("../../../protocol-artifact/poc-v1/schema.json");
const STATE_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/state-response.json");
const ACTION_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-accepted.json");
const INVALID_ACTION: &str =
    include_str!("../../../protocol-artifact/poc-v1/fixtures/invalid-action.json");
const CONFORMANCE_CASE: &str = include_str!("../../../conformance/cases/poc-v1.json");

const POC_LICENSE: &str = "MIT";
const POC_CONSUMERS: [&str; 5] = [
    "sts2-game-core",
    "sts2-game-mod",
    "sts2-gateway",
    "sts2-mcp-server",
    "sts2-harness",
];

/// Verifies the local copied artifact before the gateway POC route is exercised.
pub fn verify_poc_artifact() -> Result<(), ArtifactError> {
    let manifest = parse(MANIFEST)?;
    if manifest["artifact"] != POC_ARTIFACT
        || manifest["protocol_version"] != POC_PROTOCOL_VERSION
        || manifest["schema"] != POC_SCHEMA_SOURCE
        || manifest["schema_digest"] != POC_SCHEMA_DIGEST
        || manifest["provenance"]["source"] != POC_SCHEMA_SOURCE
        || manifest["provenance"]["generator"] != POC_GENERATOR
        || manifest["provenance"]["license"] != POC_LICENSE
        || !manifest["consumers"].as_array().is_some_and(|consumers| {
            consumers
                .iter()
                .map(Value::as_str)
                .eq(POC_CONSUMERS.into_iter().map(Some))
        })
    {
        return Err(ArtifactError::ManifestMismatch);
    }
    if parse(SOURCE_SCHEMA)?["$id"] != "sts2-poc-v1" || parse(SCHEMA)?["$id"] != "sts2-poc-v1" {
        return Err(ArtifactError::SchemaMismatch);
    }
    for fixture in [STATE_RESPONSE, ACTION_RESPONSE, INVALID_ACTION] {
        if !fixture_metadata_matches(&parse(fixture)?) {
            return Err(ArtifactError::FixtureMismatch);
        }
    }
    let conformance = parse(CONFORMANCE_CASE)?;
    if conformance["contract"] != "sts2.protocol/poc-v1"
        || conformance["profile"] != POC_PROTOCOL_VERSION
        || conformance["schema"] != POC_SCHEMA_SOURCE
    {
        return Err(ArtifactError::FixtureMismatch);
    }
    Ok(())
}

/// A deterministic failure while loading the copied artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactError {
    InvalidJson,
    ManifestMismatch,
    SchemaMismatch,
    FixtureMismatch,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("copied POC artifact is invalid")
    }
}

impl std::error::Error for ArtifactError {}

fn parse(text: &str) -> Result<Value, ArtifactError> {
    serde_json::from_str(text).map_err(|_| ArtifactError::InvalidJson)
}

fn fixture_metadata_matches(fixture: &Value) -> bool {
    fixture["protocol_version"] == POC_PROTOCOL_VERSION
        && fixture["schema_digest"] == POC_SCHEMA_DIGEST
        && fixture["provenance"]["artifact"] == POC_ARTIFACT
        && fixture["provenance"]["source"] == POC_SCHEMA_SOURCE
        && fixture["provenance"]["generator"] == POC_GENERATOR
}
