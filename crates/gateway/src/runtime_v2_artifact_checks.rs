// SPDX-License-Identifier: MIT

fn parse(bytes: &[u8]) -> Result<Value, RuntimeV2ArtifactError> {
    serde_json::from_slice(bytes).map_err(|_| RuntimeV2ArtifactError::InvalidJson)
}

fn checksum_inventory_matches_files(inventory: &[u8], files: &RuntimeV2ArtifactFiles<'_>) -> bool {
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
    let Ok(inventory) = std::str::from_utf8(inventory) else {
        return false;
    };
    let mut paths = Vec::new();
    let mut valid = true;
    for line in inventory.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            valid = false;
            continue;
        };
        let digest_shape =
            digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit());
        let known_path = expected.contains(&path);
        let unique_path = !paths.contains(&path);
        let digest_matches = bytes_for_path(path, files)
            .is_some_and(|bytes| digest_shape && sha256_hex(bytes).eq_ignore_ascii_case(digest));
        valid &= digest_shape && known_path && unique_path && digest_matches;
        paths.push(path);
    }
    valid && paths.len() == expected.len() && expected.iter().all(|path| paths.contains(path))
}

fn bytes_for_path<'a>(path: &str, files: &'a RuntimeV2ArtifactFiles<'a>) -> Option<&'a [u8]> {
    match path {
        "../../conformance/cases/runtime-v2.json" => Some(files.conformance),
        "../../schemas/runtime-v2.schema.json" => Some(files.source_schema),
        "manifest.json" => Some(files.manifest),
        "schema.json" => Some(files.artifact_schema),
        _ => files
            .goldens
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.bytes),
    }
}

fn golden_paths_match(goldens: &[RuntimeV2ArtifactFile<'_>]) -> bool {
    goldens.len() == GOLDEN_PATHS.len()
        && goldens
            .iter()
            .map(|file| file.path)
            .eq(GOLDEN_PATHS.iter().copied())
}

fn string_array_matches(value: &Value, expected: &[&str]) -> bool {
    value.as_array().is_some_and(|values| {
        values
            .iter()
            .map(Value::as_str)
            .eq(expected.iter().copied().map(Some))
    })
}
