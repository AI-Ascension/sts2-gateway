// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeConfig {
    pub(super) fn from_environment() -> Result<Self, String> {
        let listen_address = env_or_default("STS2_GATEWAY_ADDR", DEFAULT_LISTEN_ADDRESS)?;
        let mod_address = env_or_default("STS2_MOD_ADDR", DEFAULT_MOD_ADDRESS)?;
        validate_loopback_address("STS2_GATEWAY_ADDR", &listen_address)?;
        validate_loopback_address("STS2_MOD_ADDR", &mod_address)?;
        let auth_policy = AuthPolicy::from_environment()?;
        let mod_token = required("STS2_MOD_TOKEN")?;
        let instance_id = env_or_default("STS2_INSTANCE_ID", "instance-1")?;
        let caller_id = env_or_default("STS2_CALLER_ID", "harness")?;
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let mcp_session_id = env_or_default("STS2_MCP_SESSION_ID", &session_id)?;
        let lease_id = env_or_default("STS2_LEASE_ID", "lease-1")?;
        let lease_epoch = env_or_default("STS2_LEASE_EPOCH", "1")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?;
        let operation_capacity = parse_operation_capacity(&env_or_default(
            "STS2_RUNTIME_V2_OPERATION_CAPACITY",
            DEFAULT_OPERATION_CAPACITY,
        )?)?;
        let queue_capacity = parse_queue_capacity(&env_or_default(
            "STS2_RUNTIME_V2_QUEUE_CAPACITY",
            DEFAULT_QUEUE_CAPACITY,
        )?)?;
        let journal_path = optional_path("STS2_RUNTIME_V2_JOURNAL")?;
        for (name, value) in [
            ("STS2_INSTANCE_ID", &instance_id),
            ("STS2_CALLER_ID", &caller_id),
            ("STS2_SESSION_ID", &session_id),
            ("STS2_MCP_SESSION_ID", &mcp_session_id),
            ("STS2_LEASE_ID", &lease_id),
        ] {
            if !safe_identity(value) {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        for (name, value) in [("STS2_MOD_TOKEN", &mod_token)] {
            if value.is_empty()
                || value.len() > 256
                || value.bytes().any(|byte| byte.is_ascii_whitespace())
            {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        Ok(Self {
            listen_address,
            mod_address,
            auth_policy,
            mod_token,
            instance_id,
            caller_id,
            session_id,
            mcp_session_id,
            lease_id,
            lease_epoch,
            operation_capacity,
            queue_capacity,
            journal_path,
        })
    }
}

pub(super) fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

pub(super) fn validate_loopback_address(name: &str, value: &str) -> Result<(), String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| format!("{name} must be a numeric loopback IP:port endpoint"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "{name} must be a numeric loopback IP:port endpoint"
        ));
    }
    Ok(())
}

pub(super) fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn optional_path(name: &str) -> Result<Option<PathBuf>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(PathBuf::from(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn parse_operation_capacity(value: &str) -> Result<usize, String> {
    let capacity = value
        .parse::<usize>()
        .map_err(|_| String::from("STS2_RUNTIME_V2_OPERATION_CAPACITY must be an integer"))?;
    if capacity == 0 || capacity > MAX_OPERATION_CAPACITY {
        return Err(format!(
            "STS2_RUNTIME_V2_OPERATION_CAPACITY must be between 1 and {MAX_OPERATION_CAPACITY}"
        ));
    }
    Ok(capacity)
}

pub(super) fn parse_queue_capacity(value: &str) -> Result<usize, String> {
    let capacity = value
        .parse::<usize>()
        .map_err(|_| String::from("STS2_RUNTIME_V2_QUEUE_CAPACITY must be an integer"))?;
    if capacity == 0 || capacity > MAX_QUEUE_CAPACITY {
        return Err(format!(
            "STS2_RUNTIME_V2_QUEUE_CAPACITY must be between 1 and {MAX_QUEUE_CAPACITY}"
        ));
    }
    Ok(capacity)
}
