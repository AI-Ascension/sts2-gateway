// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthScope {
    Read,
    Mutate,
    Control,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AuthFailure {
    Missing,
    Invalid,
    Expired,
    Scope,
}

#[derive(Clone, Debug)]
struct Credential {
    bearer: String,
    expires_at: Option<u64>,
    scopes: u8,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthPolicy {
    current: Credential,
    previous: Option<Credential>,
}

impl AuthPolicy {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let current_token = required("STS2_GATEWAY_TOKEN")?;
        let current = Credential::from_environment(
            current_token,
            "STS2_GATEWAY_TOKEN_EXPIRES_AT",
            "STS2_GATEWAY_TOKEN_SCOPE",
            "read,mutate,control",
        )?;
        let previous = optional_token("STS2_GATEWAY_TOKEN_PREVIOUS")?
            .map(|token| {
                Credential::from_environment(
                    token,
                    "STS2_GATEWAY_TOKEN_PREVIOUS_EXPIRES_AT",
                    "STS2_GATEWAY_TOKEN_PREVIOUS_SCOPE",
                    "read,mutate,control",
                )
            })
            .transpose()?;
        if previous
            .as_ref()
            .is_some_and(|credential| credential.bearer == current.bearer)
        {
            return Err(String::from(
                "STS2_GATEWAY_TOKEN_PREVIOUS must differ from the current token",
            ));
        }
        Ok(Self { current, previous })
    }

    #[cfg(test)]
    pub(crate) fn test_all(token: &str) -> Self {
        Self {
            current: Credential {
                bearer: format!("Bearer {token}"),
                expires_at: None,
                scopes: 0b111,
            },
            previous: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn test_with_previous(
        current: &str,
        current_expires_at: Option<u64>,
        previous: Option<(&str, Option<u64>)>,
        scopes: &str,
    ) -> Result<Self, String> {
        let scope_bits = parse_scopes(scopes, "test scope")?;
        let previous = previous.map(|(token, expires_at)| Credential {
            bearer: format!("Bearer {token}"),
            expires_at,
            scopes: scope_bits,
        });
        Ok(Self {
            current: Credential {
                bearer: format!("Bearer {current}"),
                expires_at: current_expires_at,
                scopes: scope_bits,
            },
            previous,
        })
    }

    pub(crate) fn authorize(
        &self,
        provided: Option<&str>,
        scope: AuthScope,
    ) -> Result<(), AuthFailure> {
        let Some(provided) = provided else {
            return Err(AuthFailure::Missing);
        };
        let now = unix_seconds();
        let current_match =
            constant_time_equal(provided.as_bytes(), self.current.bearer.as_bytes());
        let previous_match = self.previous.as_ref().is_some_and(|credential| {
            constant_time_equal(provided.as_bytes(), credential.bearer.as_bytes())
        });
        let credential = match (current_match, previous_match, self.previous.as_ref()) {
            (true, _, _) => &self.current,
            (false, true, Some(previous)) => previous,
            _ => return Err(AuthFailure::Invalid),
        };
        if credential
            .expires_at
            .is_some_and(|expires_at| now >= expires_at)
        {
            return Err(AuthFailure::Expired);
        }
        if !credential.allows(scope) {
            return Err(AuthFailure::Scope);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn authorize_at(
        &self,
        provided: Option<&str>,
        scope: AuthScope,
        now: u64,
    ) -> Result<(), AuthFailure> {
        let Some(provided) = provided else {
            return Err(AuthFailure::Missing);
        };
        let current_match =
            constant_time_equal(provided.as_bytes(), self.current.bearer.as_bytes());
        let previous_match = self.previous.as_ref().is_some_and(|credential| {
            constant_time_equal(provided.as_bytes(), credential.bearer.as_bytes())
        });
        let credential = match (current_match, previous_match, self.previous.as_ref()) {
            (true, _, _) => &self.current,
            (false, true, Some(previous)) => previous,
            _ => return Err(AuthFailure::Invalid),
        };
        if credential
            .expires_at
            .is_some_and(|expires_at| now >= expires_at)
        {
            return Err(AuthFailure::Expired);
        }
        if !credential.allows(scope) {
            return Err(AuthFailure::Scope);
        }
        Ok(())
    }
}

impl Credential {
    fn from_environment(
        token: String,
        expiry_name: &str,
        scope_name: &str,
        default_scope: &str,
    ) -> Result<Self, String> {
        validate_token(&token, expiry_name)?;
        let expires_at = optional_u64(expiry_name)?;
        let scopes = parse_scopes(&env_or_default(scope_name, default_scope)?, scope_name)?;
        Ok(Self {
            bearer: format!("Bearer {token}"),
            expires_at,
            scopes,
        })
    }

    fn allows(&self, scope: AuthScope) -> bool {
        let required = match scope {
            AuthScope::Read => 0b001,
            AuthScope::Mutate => 0b010,
            AuthScope::Control => 0b100,
        };
        self.scopes & required != 0
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn optional_token(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn optional_u64(name: &str) -> Result<Option<u64>, String> {
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

fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn validate_token(token: &str, name: &str) -> Result<(), String> {
    if token.is_empty() || token.len() > 256 || token.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(format!("{name} is empty, unsafe, or oversized"));
    }
    Ok(())
}

fn parse_scopes(value: &str, name: &str) -> Result<u8, String> {
    let mut scopes = 0;
    for scope in value.split(',').map(str::trim) {
        let bit = match scope {
            "read" => 0b001,
            "mutate" => 0b010,
            "control" => 0b100,
            _ => return Err(format!("{name} contains an unsupported scope")),
        };
        scopes |= bit;
    }
    if scopes == 0 {
        return Err(format!("{name} must contain at least one scope"));
    }
    Ok(scopes)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = u8::from(left.len() != right.len());
    for index in 0..left.len().max(right.len()) {
        difference |=
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0);
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::{AuthFailure, AuthPolicy, AuthScope};

    #[test]
    fn current_token_requires_scope_and_expiry() -> Result<(), String> {
        let policy = AuthPolicy::test_with_previous("current", Some(100), None, "read,mutate")?;
        assert_eq!(
            policy.authorize_at(Some("Bearer current"), AuthScope::Read, 99),
            Ok(())
        );
        assert_eq!(
            policy.authorize_at(Some("Bearer current"), AuthScope::Control, 99),
            Err(AuthFailure::Scope)
        );
        assert_eq!(
            policy.authorize_at(Some("Bearer current"), AuthScope::Read, 100),
            Err(AuthFailure::Expired)
        );
        Ok(())
    }

    #[test]
    fn previous_token_is_accepted_during_rotation_until_expiry() -> Result<(), String> {
        let policy = AuthPolicy::test_with_previous(
            "current",
            Some(200),
            Some(("previous", Some(100))),
            "read",
        )?;
        assert_eq!(
            policy.authorize_at(Some("Bearer previous"), AuthScope::Read, 99),
            Ok(())
        );
        assert_eq!(
            policy.authorize_at(Some("Bearer previous"), AuthScope::Read, 100),
            Err(AuthFailure::Expired)
        );
        assert_eq!(
            policy.authorize_at(Some("Bearer missing"), AuthScope::Read, 99),
            Err(AuthFailure::Invalid)
        );
        Ok(())
    }
}
