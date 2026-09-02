// SPDX-License-Identifier: MIT

use serde_json::Value;

/// Version consumed by the gateway POC routing proof.
pub const POC_PROTOCOL_VERSION: &str = "poc-v1";
/// Schema digest supplied by the protocol release-like artifact.
pub const POC_SCHEMA_DIGEST: &str =
    "242b8f9233e915a55ea8d2e72ca476c1258169a67e62de72ee5aed848a6a0a19";
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
const POC_ARTIFACT_SCHEMA: &str = "schema.json";
const POC_MAX_GENERATION: u64 = 9_007_199_254_740_991;
const POC_CONTRACT: &str = "sts2.protocol/poc-v1";
const POC_CHECKSUM_REFERENCE: &str = "artifacts/poc-v1/SHA256SUMS";
const POC_INVALID_REFERENCE: &str = "artifacts/poc-v1/fixtures/invalid-action.json";
const POC_GOLDEN_PATHS: [&str; 5] = [
    "artifacts/poc-v1/golden/state-request.json",
    "artifacts/poc-v1/golden/state-response.json",
    "artifacts/poc-v1/golden/action-request.json",
    "artifacts/poc-v1/golden/action-accepted.json",
    "artifacts/poc-v1/golden/action-rejected.json",
];
const POC_CHECKSUM_PATHS: [&str; 10] = [
    "../../conformance/cases/poc-v1.json",
    "../../schemas/poc-v1.schema.json",
    "fixtures/invalid-action.json",
    "golden/action-accepted.json",
    "golden/action-rejected.json",
    "golden/action-request.json",
    "golden/state-request.json",
    "golden/state-response.json",
    "manifest.json",
    "schema.json",
];

const MANIFEST: &str = include_str!("../../../protocol-artifact/poc-v1/manifest.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/poc-v1/SHA256SUMS");
const SOURCE_SCHEMA: &str = include_str!("../../../schemas/poc-v1.schema.json");
const SCHEMA: &str = include_str!("../../../protocol-artifact/poc-v1/schema.json");
const STATE_REQUEST: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/state-request.json");
const STATE_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/state-response.json");
const ACTION_REQUEST: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-request.json");
const ACTION_RESPONSE: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-accepted.json");
const ACTION_REJECTED: &str =
    include_str!("../../../protocol-artifact/poc-v1/golden/action-rejected.json");
const INVALID_ACTION: &str =
    include_str!("../../../protocol-artifact/poc-v1/fixtures/invalid-action.json");
const CONFORMANCE_CASE: &str = include_str!("../../../conformance/cases/poc-v1.json");

const POC_LICENSE: &str = "MIT";
const POC_CONSUMERS: [&str; 5] = [
    "sts2-game-core",
    "sts2-game-mod",
    "sts2-gateway",
    "sts2-harness",
    "sts2-mcp-server",
];

/// Verifies the local copied artifact before the gateway POC route is exercised.
pub fn verify_poc_artifact() -> Result<(), ArtifactError> {
    let manifest = parse(MANIFEST)?;
    if manifest["artifact"] != POC_ARTIFACT
        || manifest["protocol_version"] != POC_PROTOCOL_VERSION
        || manifest["schema"] != POC_ARTIFACT_SCHEMA
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
    if !checksums_have_expected_paths(CHECKSUMS) {
        return Err(ArtifactError::ChecksumMismatch);
    }
    let source_schema = parse(SOURCE_SCHEMA)?;
    let schema = parse(SCHEMA)?;
    if SOURCE_SCHEMA.as_bytes() != SCHEMA.as_bytes()
        || source_schema["$id"] != "sts2-poc-v1"
        || schema["$id"] != "sts2-poc-v1"
        || schema["$defs"]["base"]["properties"]["generation"]["maximum"] != POC_MAX_GENERATION
    {
        return Err(ArtifactError::SchemaMismatch);
    }
    for fixture in [
        STATE_REQUEST,
        STATE_RESPONSE,
        ACTION_REQUEST,
        ACTION_RESPONSE,
        ACTION_REJECTED,
        INVALID_ACTION,
    ] {
        if !fixture_metadata_matches(&parse(fixture)?) {
            return Err(ArtifactError::FixtureMismatch);
        }
    }
    let conformance = parse(CONFORMANCE_CASE)?;
    if conformance["case_id"] != "CT-POC-V1-001"
        || conformance["contract"] != POC_CONTRACT
        || conformance["profile"] != POC_PROTOCOL_VERSION
        || conformance["schema"] != POC_SCHEMA_SOURCE
        || conformance["invalid"] != POC_INVALID_REFERENCE
        || conformance["checksums"] != POC_CHECKSUM_REFERENCE
        || !string_array_matches(&conformance["goldens"], &POC_GOLDEN_PATHS)
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
    ChecksumMismatch,
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

fn checksums_have_expected_paths(inventory: &str) -> bool {
    let mut paths = Vec::new();
    for line in inventory.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        paths.push(path);
    }
    paths.as_slice() == POC_CHECKSUM_PATHS
}

fn string_array_matches(value: &Value, expected: &[&str]) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .map(Value::as_str)
            .eq(expected.iter().copied().map(Some))
    })
}
