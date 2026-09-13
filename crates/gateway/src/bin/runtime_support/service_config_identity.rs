// SPDX-License-Identifier: MIT

use super::safe_identity;

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
