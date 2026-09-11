// SPDX-License-Identifier: MIT

pub(super) fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

pub(super) fn optional_token(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn optional_u64(name: &str) -> Result<Option<u64>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{name} must be an unsigned Unix timestamp")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

pub(super) fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
