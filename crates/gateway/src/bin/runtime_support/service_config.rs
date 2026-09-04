// SPDX-License-Identifier: MIT

use super::safe_identity;
use std::net::SocketAddr;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:15525";
const DEFAULT_MOD_ADDRESS: &str = "127.0.0.1:15526";

pub(super) struct RuntimeConfig {
    pub(super) listen_address: String,
    pub(super) mod_address: String,
    pub(super) gateway_token: String,
    pub(super) mod_token: String,
    pub(super) instance_id: String,
    pub(super) caller_id: String,
    pub(super) session_id: String,
    pub(super) lease_id: String,
    pub(super) lease_epoch: u64,
}

impl RuntimeConfig {
    pub(super) fn from_environment() -> Result<Self, String> {
        let listen_address = env_or_default("STS2_GATEWAY_ADDR", DEFAULT_LISTEN_ADDRESS)?;
        let mod_address = env_or_default("STS2_MOD_ADDR", DEFAULT_MOD_ADDRESS)?;
        validate_loopback_address(&listen_address)?;
        validate_loopback_address(&mod_address)?;
        let gateway_token = required("STS2_GATEWAY_TOKEN")?;
        let mod_token = required("STS2_MOD_TOKEN")?;
        let instance_id = env_or_default("STS2_INSTANCE_ID", "instance-1")?;
        let caller_id = env_or_default("STS2_CALLER_ID", "harness")?;
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let lease_id = env_or_default("STS2_LEASE_ID", "lease-1")?;
        let lease_epoch = env_or_default("STS2_LEASE_EPOCH", "1")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?;
        for (name, value) in [
            ("STS2_INSTANCE_ID", &instance_id),
            ("STS2_CALLER_ID", &caller_id),
            ("STS2_SESSION_ID", &session_id),
            ("STS2_LEASE_ID", &lease_id),
        ] {
            if !safe_identity(value) {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        for (name, value) in [
            ("STS2_GATEWAY_TOKEN", &gateway_token),
            ("STS2_MOD_TOKEN", &mod_token),
        ] {
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
            gateway_token,
            mod_token,
            instance_id,
            caller_id,
            session_id,
            lease_id,
            lease_epoch,
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

pub(super) fn validate_loopback_address(value: &str) -> Result<(), String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| String::from("runtime addresses must be literal loopback IP:port"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(String::from(
            "runtime addresses must use loopback and a nonzero port",
        ));
    }
    Ok(())
}

fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
