// SPDX-License-Identifier: MIT

#[path = "runtime_v2_artifact_checksum.rs"]
mod runtime_v2_artifact_checksum;

use serde_json::Value;

use runtime_v2_artifact_checksum::sha256_hex;

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

/// One copied artifact file supplied to the byte-level verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeV2ArtifactFile<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// The copied Runtime-v2 files verified as one release-like artifact.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeV2ArtifactFiles<'a> {
    pub source_schema: &'a [u8],
    pub artifact_schema: &'a [u8],
    pub manifest: &'a [u8],
    pub checksums: &'a [u8],
    pub conformance: &'a [u8],
    pub goldens: &'a [RuntimeV2ArtifactFile<'a>],
}

const ARTIFACT_SCHEMA: &[u8] = include_bytes!("../../../protocol-artifact/runtime-v2/schema.json");
const CHECKSUMS: &[u8] = include_bytes!("../../../protocol-artifact/runtime-v2/SHA256SUMS");
const CONFORMANCE_CASE: &[u8] = include_bytes!("../../../conformance/cases/runtime-v2.json");
const MANIFEST: &[u8] = include_bytes!("../../../protocol-artifact/runtime-v2/manifest.json");
const SOURCE_SCHEMA: &[u8] = include_bytes!("../../../schemas/runtime-v2.schema.json");

const GOLDENS: [RuntimeV2ArtifactFile<'static>; 19] = [
    RuntimeV2ArtifactFile {
        path: "golden/state-request.json",
        bytes: include_bytes!("../../../protocol-artifact/runtime-v2/golden/state-request.json"),
    },
    RuntimeV2ArtifactFile {
        path: "golden/state-response.json",
        bytes: include_bytes!("../../../protocol-artifact/runtime-v2/golden/state-response.json"),
    },
    RuntimeV2ArtifactFile {
        path: "golden/legal-action-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/legal-action-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/legal-action-accepted.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/legal-action-accepted.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/legal-action-settled.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/legal-action-settled.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/stale-generation-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/stale-generation-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/stale-generation-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/stale-generation-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/outside-combat-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/outside-combat-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/outside-combat-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/outside-combat-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/enemy-turn-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/enemy-turn-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/enemy-turn-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/enemy-turn-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/idempotency-conflict-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/idempotency-conflict-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/idempotency-conflict-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/cancelled-before-dispatch.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/cancelled-before-dispatch.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/timeout-action-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/timeout-action-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/timeout-unknown-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/timeout-unknown-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/reconcile-request.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/reconcile-request.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/reconcile-settled-response.json",
        bytes: include_bytes!(
            "../../../protocol-artifact/runtime-v2/golden/reconcile-settled-response.json"
        ),
    },
    RuntimeV2ArtifactFile {
        path: "golden/duplicate-replay.json",
        bytes: include_bytes!("../../../protocol-artifact/runtime-v2/golden/duplicate-replay.json"),
    },
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

/// Returns the release-like copied files used by the gateway verifier.
#[must_use]
pub fn runtime_v2_artifact_files() -> RuntimeV2ArtifactFiles<'static> {
    RuntimeV2ArtifactFiles {
        source_schema: SOURCE_SCHEMA,
        artifact_schema: ARTIFACT_SCHEMA,
        manifest: MANIFEST,
        checksums: CHECKSUMS,
        conformance: CONFORMANCE_CASE,
        goldens: &GOLDENS,
    }
}

/// Verifies the copied Runtime-v2 artifact before a v2 adapter is used.
pub fn verify_runtime_v2_artifact() -> Result<(), RuntimeV2ArtifactError> {
    verify_runtime_v2_artifact_files(runtime_v2_artifact_files())
}

/// Verifies supplied release-like bytes, including every listed SHA-256.
pub fn verify_runtime_v2_artifact_files(
    files: RuntimeV2ArtifactFiles<'_>,
) -> Result<(), RuntimeV2ArtifactError> {
    if !checksum_inventory_matches_files(files.checksums, &files) {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
    }
    let manifest = parse(files.manifest)?;
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

    if files.source_schema != files.artifact_schema {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }
    let source = parse(files.source_schema)?;
    if source["$id"] != "sts2-runtime-v2"
        || source["$defs"]["action"]["properties"]["action_id"]["const"] != RUNTIME_V2_ACTION_ID
        || source["$defs"]["observation"]["properties"]["turn_index"]["maximum"]
            != RUNTIME_V2_MAX_TURN_INDEX
    {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }

    if !golden_paths_match(files.goldens) {
        return Err(RuntimeV2ArtifactError::FixtureMismatch);
    }
    for golden in files.goldens {
        if parse(golden.bytes).is_err() {
            return Err(RuntimeV2ArtifactError::FixtureMismatch);
        }
    }
    let conformance = parse(files.conformance)?;
    if conformance["case_id"] != "CT-RUNTIME-V2-001"
        || conformance["profile"] != RUNTIME_V2_PROTOCOL_VERSION
        || conformance["schema"] != RUNTIME_V2_SCHEMA_SOURCE
        || conformance["checksums"] != "artifacts/runtime-v2/SHA256SUMS"
        || conformance["contract_assertions"]["action_id"] != RUNTIME_V2_ACTION_ID
        || conformance["contract_assertions"]["settlement_witness"] != RUNTIME_V2_EFFECT_KIND
    {
        return Err(RuntimeV2ArtifactError::FixtureMismatch);
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

include!("runtime_v2_artifact_checks.rs");
