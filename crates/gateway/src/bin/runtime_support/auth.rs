// SPDX-License-Identifier: MIT

use std::time::{SystemTime, UNIX_EPOCH};

#[path = "auth_environment.rs"]
mod environment;

use environment::{env_or_default, optional_token, optional_u64, required};

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
    recovery_current: Option<Credential>,
    recovery_previous: Option<Credential>,
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
        let recovery_current = optional_token("STS2_RECOVERY_TOKEN")?
            .map(|token| {
                Credential::from_environment(
                    token,
                    "STS2_RECOVERY_TOKEN_EXPIRES_AT",
                    "STS2_RECOVERY_TOKEN_SCOPE",
                    "read,mutate,control",
                )
            })
            .transpose()?;
        let recovery_previous = optional_token("STS2_RECOVERY_TOKEN_PREVIOUS")?
            .map(|token| {
                Credential::from_environment(
                    token,
                    "STS2_RECOVERY_TOKEN_PREVIOUS_EXPIRES_AT",
                    "STS2_RECOVERY_TOKEN_PREVIOUS_SCOPE",
                    "read,mutate,control",
                )
            })
            .transpose()?;
        if recovery_previous.is_some() && recovery_current.is_none() {
            return Err(String::from(
                "STS2_RECOVERY_TOKEN is required when a previous recovery token is configured",
            ));
        }
        if recovery_previous.as_ref().is_some_and(|credential| {
            recovery_current
                .as_ref()
                .is_some_and(|current| current.bearer == credential.bearer)
        }) {
            return Err(String::from(
                "STS2_RECOVERY_TOKEN_PREVIOUS must differ from the current recovery token",
            ));
        }
        Ok(Self {
            current,
            previous,
            recovery_current,
            recovery_previous,
        })
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
            recovery_current: Some(Credential {
                bearer: format!("Bearer {token}"),
                expires_at: None,
                scopes: 0b111,
            }),
            recovery_previous: None,
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
            recovery_current: Some(Credential {
                bearer: format!("Bearer {current}"),
                expires_at: current_expires_at,
                scopes: scope_bits,
            }),
            recovery_previous: previous.clone(),
            previous,
        })
    }

    pub(crate) fn authorize(
        &self,
        provided: Option<&str>,
        scope: AuthScope,
    ) -> Result<(), AuthFailure> {
        authorize_credentials(
            provided,
            scope,
            unix_seconds(),
            &self.current,
            self.previous.as_ref(),
        )
    }

    pub(crate) fn authorize_recovery(
        &self,
        provided: Option<&str>,
        scope: AuthScope,
    ) -> Result<(), AuthFailure> {
        let Some(current) = self.recovery_current.as_ref() else {
            return Err(AuthFailure::Missing);
        };
        authorize_credentials(
            provided,
            scope,
            unix_seconds(),
            current,
            self.recovery_previous.as_ref(),
        )
    }

    #[cfg(test)]
    pub(crate) fn authorize_at(
        &self,
        provided: Option<&str>,
        scope: AuthScope,
        now: u64,
    ) -> Result<(), AuthFailure> {
        authorize_credentials(provided, scope, now, &self.current, self.previous.as_ref())
    }
}

fn authorize_credentials(
    provided: Option<&str>,
    scope: AuthScope,
    now: u64,
    current: &Credential,
    previous: Option<&Credential>,
) -> Result<(), AuthFailure> {
    let Some(provided) = provided else {
        return Err(AuthFailure::Missing);
    };
    let current_match = constant_time_equal(provided.as_bytes(), current.bearer.as_bytes());
    let previous_match = previous.is_some_and(|credential| {
        constant_time_equal(provided.as_bytes(), credential.bearer.as_bytes())
    });
    let credential = match (current_match, previous_match, previous) {
        (true, _, _) => current,
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
#[path = "auth_tests.rs"]
mod tests;
