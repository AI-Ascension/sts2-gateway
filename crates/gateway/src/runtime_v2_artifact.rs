// SPDX-License-Identifier: MIT

use serde_json::Value;

/// The Runtime-v2 protocol version consumed by this target.
pub const RUNTIME_V2_PROTOCOL_VERSION: &str = "runtime-v2";
/// The release-like artifact identity carried by Runtime-v2 messages.
pub const RUNTIME_V2_ARTIFACT: &str = "sts2-protocol/runtime-v2";
/// The canonical schema source recorded in the artifact provenance.
pub const RUNTIME_V2_SCHEMA_SOURCE: &str = "schemas/runtime-v2.schema.json";
/// The generator recorded by the protocol captain for this artifact.
pub const RUNTIME_V2_GENERATOR: &str = "hand-authored";
/// SHA-256 of the handed-off Runtime-v2 schema source bytes.
pub const RUNTIME_V2_SCHEMA_DIGEST: &str =
    "f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2";
/// The only mutation admitted by Runtime-v2.
pub const RUNTIME_V2_ACTION_ID: &str = "end_turn";
/// The witness required for a settled end-turn operation.
pub const RUNTIME_V2_EFFECT_KIND: &str = "turn_end_settled";
/// The only combat phase in which the fixed action is admissible.
pub const RUNTIME_V2_PLAYER_TURN_PHASE: &str = "combat/player_turn";
/// Maximum generation and lease epoch represented exactly by the contract.
pub const RUNTIME_V2_MAX_GENERATION: u64 = 9_007_199_254_740_991;
/// Maximum turn index represented by the contract.
pub const RUNTIME_V2_MAX_TURN_INDEX: u16 = 1024;

const ARTIFACT_SCHEMA: &str = include_str!("../../../protocol-artifact/runtime-v2/schema.json");
const CHECKSUMS: &str = include_str!("../../../protocol-artifact/runtime-v2/SHA256SUMS");
const CONFORMANCE_CASE: &str = include_str!("../../../conformance/cases/runtime-v2.json");
const MANIFEST: &str = include_str!("../../../protocol-artifact/runtime-v2/manifest.json");
const SOURCE_SCHEMA: &str = include_str!("../../../schemas/runtime-v2.schema.json");

const GOLDENS: [&str; 19] = [
    include_str!("../../../protocol-artifact/runtime-v2/golden/state-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/state-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/legal-action-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/legal-action-accepted.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/legal-action-settled.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/stale-generation-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/stale-generation-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/outside-combat-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/outside-combat-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/enemy-turn-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/enemy-turn-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/cancelled-before-dispatch.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/timeout-action-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/timeout-unknown-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/reconcile-request.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/reconcile-settled-response.json"),
    include_str!("../../../protocol-artifact/runtime-v2/golden/duplicate-replay.json"),
];

const GOLDEN_PATHS: [&str; 19] = [
    "golden/state-request.json",
    "golden/state-response.json",
    "golden/legal-action-request.json",
    "golden/legal-action-accepted.json",
    "golden/legal-action-settled.json",
    "golden/stale-generation-request.json",
    "golden/stale-generation-response.json",
    "golden/outside-combat-request.json",
    "golden/outside-combat-response.json",
    "golden/enemy-turn-request.json",
    "golden/enemy-turn-response.json",
    "golden/idempotency-conflict-request.json",
    "golden/idempotency-conflict-response.json",
    "golden/cancelled-before-dispatch.json",
    "golden/timeout-action-request.json",
    "golden/timeout-unknown-response.json",
    "golden/reconcile-request.json",
    "golden/reconcile-settled-response.json",
    "golden/duplicate-replay.json",
];

/// Verifies the copied Runtime-v2 artifact before a v2 adapter is used.
pub fn verify_runtime_v2_artifact() -> Result<(), RuntimeV2ArtifactError> {
    let manifest = parse(MANIFEST)?;
    if manifest["artifact"] != RUNTIME_V2_ARTIFACT
        || manifest["protocol_version"] != RUNTIME_V2_PROTOCOL_VERSION
        || manifest["schema"] != "schema.json"
        || manifest["schema_digest"] != RUNTIME_V2_SCHEMA_DIGEST
        || manifest["provenance"]["source"] != RUNTIME_V2_SCHEMA_SOURCE
        || manifest["provenance"]["generator"] != RUNTIME_V2_GENERATOR
        || manifest["provenance"]["license"] != "MIT"
        || manifest["checksums"] != "SHA256SUMS"
        || !string_array_matches(&manifest["goldens"], &GOLDEN_PATHS)
        || !string_array_matches(
            &manifest["consumers"],
            &[
                "sts2-game-mod",
                "sts2-gateway",
                "sts2-harness",
                "sts2-mcp-server",
            ],
        )
    {
        return Err(RuntimeV2ArtifactError::ManifestMismatch);
    }

    if SOURCE_SCHEMA.as_bytes() != ARTIFACT_SCHEMA.as_bytes() {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }
    let source = parse(SOURCE_SCHEMA)?;
    if source["$id"] != "sts2-runtime-v2"
        || source["$defs"]["action"]["properties"]["action_id"]["const"] != RUNTIME_V2_ACTION_ID
        || source["$defs"]["observation"]["properties"]["turn_index"]["maximum"]
            != RUNTIME_V2_MAX_TURN_INDEX
    {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }

    for golden in GOLDENS {
        if parse(golden).is_err() {
            return Err(RuntimeV2ArtifactError::FixtureMismatch);
        }
    }
    let conformance = parse(CONFORMANCE_CASE)?;
    if conformance["case_id"] != "CT-RUNTIME-V2-001"
        || conformance["profile"] != RUNTIME_V2_PROTOCOL_VERSION
        || conformance["schema"] != RUNTIME_V2_SCHEMA_SOURCE
        || conformance["checksums"] != "artifacts/runtime-v2/SHA256SUMS"
        || conformance["contract_assertions"]["action_id"] != RUNTIME_V2_ACTION_ID
        || conformance["contract_assertions"]["settlement_witness"] != RUNTIME_V2_EFFECT_KIND
    {
        return Err(RuntimeV2ArtifactError::FixtureMismatch);
    }
    if !checksum_inventory_has_expected_paths(CHECKSUMS) {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
    }
    Ok(())
}

/// A deterministic failure while loading the copied Runtime-v2 artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2ArtifactError {
    InvalidJson,
    ManifestMismatch,
    SchemaMismatch,
    FixtureMismatch,
    ChecksumMismatch,
}

impl std::fmt::Display for RuntimeV2ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("copied Runtime-v2 artifact is invalid")
    }
}

impl std::error::Error for RuntimeV2ArtifactError {}

fn parse(text: &str) -> Result<Value, RuntimeV2ArtifactError> {
    serde_json::from_str(text).map_err(|_| RuntimeV2ArtifactError::InvalidJson)
}

fn checksum_inventory_has_expected_paths(inventory: &str) -> bool {
    let expected = [
        "../../conformance/cases/runtime-v2.json",
        "../../schemas/runtime-v2.schema.json",
        "manifest.json",
        "schema.json",
        "golden/cancelled-before-dispatch.json",
        "golden/duplicate-replay.json",
        "golden/enemy-turn-request.json",
        "golden/enemy-turn-response.json",
        "golden/idempotency-conflict-request.json",
        "golden/idempotency-conflict-response.json",
        "golden/legal-action-accepted.json",
        "golden/legal-action-request.json",
        "golden/legal-action-settled.json",
        "golden/outside-combat-request.json",
        "golden/outside-combat-response.json",
        "golden/reconcile-request.json",
        "golden/reconcile-settled-response.json",
        "golden/stale-generation-request.json",
        "golden/stale-generation-response.json",
        "golden/state-request.json",
        "golden/state-response.json",
        "golden/timeout-action-request.json",
        "golden/timeout-unknown-response.json",
    ];
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
    paths.as_slice() == expected
        && inventory.lines().any(|line| {
            line.ends_with("  ../../schemas/runtime-v2.schema.json")
                && line.starts_with(RUNTIME_V2_SCHEMA_DIGEST)
        })
        && inventory.lines().any(|line| {
            line.ends_with("  schema.json") && line.starts_with(RUNTIME_V2_SCHEMA_DIGEST)
        })
}

fn string_array_matches(value: &Value, expected: &[&str]) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .map(Value::as_str)
            .eq(expected.iter().copied().map(Some))
    })
}
