// SPDX-License-Identifier: MIT

//! Pinned schemas and strict canonical-frame validation for exact-restore transport.

use std::sync::OnceLock;

use serde_json::Value;

const WRAPPER_SCHEMA: &str =
    include_str!("../../../../../protocol-artifact/exact-restore-gateway-v1/schema.json");
const NEUTRAL_SCHEMA: &str =
    include_str!("../../../../../protocol-artifact/exact-restore-v1/schema.json");

pub(super) fn canonical_bytes(value: &Value, bytes: &[u8]) -> bool {
    serde_json::to_vec(value).is_ok_and(|canonical| canonical == bytes)
}

pub(super) fn valid_wrapper_schema(value: &Value) -> bool {
    wrapper_validator().is_some_and(|validator| validator.is_valid(value))
}

pub(super) fn valid_neutral_schema(value: &Value) -> bool {
    neutral_validator().is_some_and(|validator| validator.is_valid(value))
}

fn wrapper_validator() -> Option<&'static jsonschema::Validator> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| schema_validator(WRAPPER_SCHEMA))
        .as_ref()
}

fn neutral_validator() -> Option<&'static jsonschema::Validator> {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| schema_validator(NEUTRAL_SCHEMA))
        .as_ref()
}

fn schema_validator(schema: &str) -> Option<jsonschema::Validator> {
    let value: Value = super::super::super::super::strict_json::parse(schema.as_bytes()).ok()?;
    jsonschema::validator_for(&value).ok()
}
