// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};

const CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/dependency-semantics-v1.json"
));

#[test]
fn pinned_dependencies_preserve_shared_schema_and_sha256_semantics() -> Result<(), String> {
    let corpus: Value = serde_json::from_str(CORPUS).map_err(|error| error.to_string())?;
    if corpus["corpus"] != "sts2-dependency-semantics-v1" {
        return Err(String::from("unexpected dependency semantic corpus"));
    }
    let input = corpus["sha256_input"]
        .as_str()
        .ok_or("missing SHA-256 input")?;
    let expected = corpus["sha256"].as_str().ok_or("missing SHA-256 digest")?;
    if format!("{:x}", Sha256::digest(input.as_bytes())) != expected {
        return Err(String::from("SHA-256 semantic vector mismatch"));
    }
    let validator =
        jsonschema::validator_for(&corpus["schema"]).map_err(|error| error.to_string())?;
    for case in corpus["cases"].as_array().ok_or("missing semantic cases")? {
        let name = case["name"].as_str().ok_or("case without name")?;
        let expected = case["valid"].as_bool().ok_or("case without expectation")?;
        if validator.is_valid(&case["value"]) != expected {
            return Err(format!("schema semantic mismatch: {name}"));
        }
    }
    Ok(())
}
