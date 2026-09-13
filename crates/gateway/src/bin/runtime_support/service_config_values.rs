// SPDX-License-Identifier: MIT

use super::*;
use uuid::{Uuid, Variant};

pub(super) fn parse_bool(name: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{name} must be true or false")),
    }
}

pub(crate) fn configured_mcp_session(
    value: Result<String, std::env::VarError>,
) -> Result<String, String> {
    let session = match value {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::from("mcp-session-1"),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(String::from("STS2_MCP_SESSION_ID is not valid UTF-8"));
        }
    };
    if !safe_identity(&session) {
        return Err(String::from(
            "STS2_MCP_SESSION_ID is empty, unsafe, or oversized",
        ));
    }
    Ok(session)
}

pub(super) fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value && id.get_variant() == Variant::RFC4122
    })
}

pub(super) fn valid_uuid_v4(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value
            && id.get_variant() == Variant::RFC4122
            && id.get_version_num() == 4
    })
}

pub(super) fn parse_recovery_seconds(
    name: &str,
    default: &str,
    minimum: u64,
    maximum: u64,
) -> Result<u64, String> {
    let value = env_or_default(name, default)?
        .parse::<u64>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!("{name} must be between {minimum} and {maximum}"));
    }
    Ok(value)
}
