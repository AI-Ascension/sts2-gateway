// SPDX-License-Identifier: MIT

use std::fmt;

use uuid::{Uuid, Variant};

use super::recovery_types::MAX_WIRE_INTEGER;

#[derive(Debug, Eq, PartialEq)]
pub enum RecoveryStoreError {
    Busy,
    Io(String),
    Sql(String),
    Corrupt(String),
    IncompatibleSchema { found: i64, expected: i64 },
    InvalidInput(String),
    ContractMismatch(String),
    ReleaseMismatch,
    AuthorityBlocked,
    AuthorityNotFound,
    CounterExhausted,
    HostFenceRequired,
    LeaseNotFound,
    LeaseExpired,
    LeaseRevoked,
    StaleLease,
    InvalidTransition,
    OperationNotFound,
    OperationConflict,
    AdmissionTicketNotFound,
    AdmissionTicketExpired,
    CapacityExceeded,
    PersistenceUnavailable,
    BackupExists,
}

impl fmt::Display for RecoveryStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("recovery store is busy"),
            Self::Io(message) | Self::Sql(message) | Self::Corrupt(message) => {
                formatter.write_str(message)
            }
            Self::IncompatibleSchema { found, expected } => {
                write!(
                    formatter,
                    "recovery schema {found} is incompatible with {expected}"
                )
            }
            Self::InvalidInput(message) | Self::ContractMismatch(message) => {
                formatter.write_str(message)
            }
            Self::ReleaseMismatch => formatter.write_str("approved release digests do not match"),
            Self::AuthorityBlocked => formatter.write_str("recovery authority is blocked"),
            Self::AuthorityNotFound => formatter.write_str("recovery authority is not initialized"),
            Self::CounterExhausted => formatter.write_str("recovery wire counter is exhausted"),
            Self::HostFenceRequired => formatter.write_str("host fence handshake is required"),
            Self::LeaseNotFound => formatter.write_str("lease was not found"),
            Self::LeaseExpired => formatter.write_str("lease has expired"),
            Self::LeaseRevoked => formatter.write_str("lease was revoked"),
            Self::StaleLease => formatter.write_str("lease proof is stale"),
            Self::InvalidTransition => formatter.write_str("operation transition is invalid"),
            Self::OperationNotFound => formatter.write_str("operation was not found"),
            Self::OperationConflict => {
                formatter.write_str("operation payload or context conflicts")
            }
            Self::AdmissionTicketNotFound => formatter.write_str("admission ticket was not found"),
            Self::AdmissionTicketExpired => formatter.write_str("admission ticket has expired"),
            Self::CapacityExceeded => {
                formatter.write_str("unresolved operation capacity is exhausted")
            }
            Self::PersistenceUnavailable => {
                formatter.write_str("recovery persistence is unavailable")
            }
            Self::BackupExists => formatter.write_str("backup destination already exists"),
        }
    }
}

impl std::error::Error for RecoveryStoreError {}

pub(crate) fn validate_identity(name: &str, value: &str) -> Result<(), RecoveryStoreError> {
    if value.is_empty() || value.len() > 512 || value.contains("..") {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} is empty, unsafe, or oversized"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
    {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} contains an unsupported character"
        )));
    }
    Ok(())
}

pub(crate) fn validate_uuid(name: &str, value: &str) -> Result<(), RecoveryStoreError> {
    validate_uuid_kind(name, value, false)
}

pub(crate) fn validate_uuid_v4(name: &str, value: &str) -> Result<(), RecoveryStoreError> {
    validate_uuid_kind(name, value, true)
}

fn validate_uuid_kind(name: &str, value: &str, v4: bool) -> Result<(), RecoveryStoreError> {
    let parsed = Uuid::parse_str(value).map_err(|_| invalid_uuid(name))?;
    if parsed.get_variant() != Variant::RFC4122
        || parsed.hyphenated().to_string() != value
        || (v4 && parsed.get_version_num() != 4)
    {
        return Err(invalid_uuid(name));
    }
    Ok(())
}

fn invalid_uuid(name: &str) -> RecoveryStoreError {
    RecoveryStoreError::InvalidInput(format!("{name} must be a lowercase RFC-4122 UUID"))
}

pub(crate) fn validate_token(name: &str, value: &str) -> Result<(), RecoveryStoreError> {
    if value.len() != 43
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} must be a bounded transport token"
        )));
    }
    Ok(())
}

pub(crate) fn validate_digest(name: &str, value: &str) -> Result<(), RecoveryStoreError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} must be a SHA-256 digest"
        )));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} must be lowercase"
        )));
    }
    Ok(())
}

pub(crate) fn validate_wire(value: u64, name: &str) -> Result<(), RecoveryStoreError> {
    if value > MAX_WIRE_INTEGER {
        return Err(RecoveryStoreError::InvalidInput(format!(
            "{name} exceeds the JSON wire integer bound"
        )));
    }
    Ok(())
}
