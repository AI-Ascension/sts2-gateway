// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::sync::OnceLock;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-v4-expert/schema.json"
));

pub(super) fn observation_valid(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref()
        .is_some_and(|validator| {
            value["protocol_version"] == "runtime-v4-expert"
                && value["schema_digest"]
                    == "0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42"
                && validator.is_valid(value)
        })
}
