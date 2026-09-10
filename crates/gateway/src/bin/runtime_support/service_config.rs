// SPDX-License-Identifier: MIT

use super::super::host_lease_control::host_lease_key_from_environment;
use super::*;
use uuid::{Uuid, Variant};

#[path = "service_config_runtime_v2.rs"]
mod runtime_v2;
pub(super) use runtime_v2::build_runtime_v2;

pub(super) fn coop_reports_from_environment() -> Result<Option<CoopReports>, String> {
    match std::env::var("STS2_COOP_ROSTER") {
        Ok(text) => CoopReports::from_roster(&text).map(Some),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err("STS2_COOP_ROSTER is not UTF-8".to_owned()),
    }
}

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
        let mcp_session_id = configured_mcp_session(std::env::var("STS2_MCP_SESSION_ID"))?;
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
        let recovery_store_path = optional_path("STS2_RECOVERY_STORE")?;
        let recovery_deployment_id = match recovery_store_path.as_ref() {
            Some(_) => Some(env_or_default(
                "STS2_DEPLOYMENT_ID",
                "00000000-0000-4000-8000-000000000001",
            )?),
            None => None,
        };
        let recovery_profile = matches!(
            std::env::var("STS2_RUNTIME_PROFILE"),
            Ok(value) if value == "watchdog-recovery-v1"
        );
        let zero_digest = "0".repeat(64);
        let recovery_release = RecoveryReleaseSet::new(
            &env_or_default("STS2_RECOVERY_RELEASE_DIGEST", &zero_digest)?,
            &env_or_default("STS2_RECOVERY_CONFIG_DIGEST", &zero_digest)?,
            &env_or_default("STS2_RECOVERY_PROFILE_DIGEST", &zero_digest)?,
            &env_or_default(
                "STS2_RECOVERY_RUNTIME_V3_SCHEMA_DIGEST",
                sts2_gateway::RUNTIME_V3_SCHEMA_DIGEST,
            )?,
        )
        .map_err(|error| format!("recovery release is invalid: {error}"))?;
        let recovery_ttl_seconds =
            parse_recovery_seconds("STS2_RECOVERY_LEASE_TTL_SECONDS", "30", 5, 300)?;
        let recovery_renewal_interval_seconds =
            parse_recovery_seconds("STS2_RECOVERY_LEASE_RENEWAL_INTERVAL_SECONDS", "10", 1, 100)?;
        let (host_lease_key, host_principal_id) = if recovery_profile
            || recovery_store_path.is_some()
        {
            let host_principal_id = required("STS2_RUNTIME_HOST_PRINCIPAL_ID")?;
            if !valid_uuid(&host_principal_id) {
                return Err(String::from(
                    "STS2_RUNTIME_HOST_PRINCIPAL_ID must be a lowercase RFC-4122 UUID",
                ));
            }
            let host_lease_key = host_lease_key_from_environment()
                .map_err(|_| String::from("STS2_RUNTIME_HOST_LEASE_KEY must encode 32 bytes"))?;
            (host_lease_key, host_principal_id)
        } else {
            (Vec::new(), String::new())
        };
        let workflow_boot_epoch = optional_value("STS2_WORKFLOW_BOOT_EPOCH")?;
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
        if let Some(deployment_id) = recovery_deployment_id.as_ref()
            && !safe_identity(deployment_id)
        {
            return Err(String::from(
                "STS2_DEPLOYMENT_ID is empty, unsafe, or oversized",
            ));
        }
        if recovery_profile || recovery_store_path.is_some() {
            if !valid_uuid(&instance_id) {
                return Err(String::from(
                    "STS2_INSTANCE_ID must be a lowercase RFC-4122 UUID for the recovery profile",
                ));
            }
            if !valid_uuid(&caller_id) {
                return Err(String::from(
                    "STS2_CALLER_ID must be a lowercase RFC-4122 UUID for the recovery profile",
                ));
            }
            if !valid_uuid_v4(&session_id) {
                return Err(String::from(
                    "STS2_SESSION_ID must be a lowercase UUIDv4 for the recovery profile",
                ));
            }
            if let Some(deployment_id) = recovery_deployment_id.as_ref()
                && !valid_uuid(deployment_id)
            {
                return Err(String::from(
                    "STS2_DEPLOYMENT_ID must be a lowercase RFC-4122 UUID for the recovery profile",
                ));
            }
            match std::env::var("STS2_RECOVERY_PRINCIPAL_ID") {
                Ok(principal) if principal != caller_id || !valid_uuid(&principal) => {
                    return Err(String::from(
                        "STS2_CALLER_ID must match STS2_RECOVERY_PRINCIPAL_ID",
                    ));
                }
                Ok(_) | Err(std::env::VarError::NotPresent) => {}
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err(String::from(
                        "STS2_RECOVERY_PRINCIPAL_ID is not valid UTF-8",
                    ));
                }
            }
        }
        if let Some(value) = workflow_boot_epoch.as_deref()
            && !safe_identity(value)
        {
            return Err(String::from(
                "STS2_WORKFLOW_BOOT_EPOCH is empty, unsafe, or oversized",
            ));
        }
        let workflow_authority = workflow_boot_epoch
            .as_deref()
            .map(|boot_epoch| {
                RuntimeV2Authority::new(
                    &instance_id,
                    &session_id,
                    &lease_id,
                    lease_epoch,
                    boot_epoch,
                )
                .map_err(|error| format!("STS2_WORKFLOW_BOOT_EPOCH is invalid: {error}"))
            })
            .transpose()?;
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
            recovery_store_path,
            recovery_deployment_id,
            recovery_release,
            recovery_ttl_seconds,
            recovery_renewal_interval_seconds,
            host_lease_key,
            host_principal_id,
            workflow_authority,
        })
    }
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value && id.get_variant() == Variant::RFC4122
    })
}

fn valid_uuid_v4(value: &str) -> bool {
    Uuid::parse_str(value).ok().is_some_and(|id| {
        id.hyphenated().to_string() == value
            && id.get_variant() == Variant::RFC4122
            && id.get_version_num() == 4
    })
}

fn parse_recovery_seconds(
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

pub(super) fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

pub(super) fn validate_loopback_address(name: &str, value: &str) -> Result<(), String> {
    let address = value
        .parse::<SocketAddr>()
        .map_err(|_| format!("{name} must be a numeric loopback IP:port endpoint"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(format!(
            "{name} must be a numeric loopback IP:port endpoint with a nonzero port"
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

pub(super) fn optional_value(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(value)),
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

pub(super) fn configured_mcp_session(
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
