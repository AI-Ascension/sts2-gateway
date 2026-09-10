// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GatewayConfig {
    capacity: usize,
    lease_duration_millis: u64,
    max_body_bytes: usize,
    max_response_bytes: usize,
}

impl GatewayConfig {
    /// Creates configuration without I/O; production callers should prefer `try_new`.
    pub const fn new(
        capacity: usize,
        lease_duration_millis: u64,
        max_body_bytes: usize,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            capacity,
            lease_duration_millis,
            max_body_bytes,
            max_response_bytes,
        }
    }

    /// Rejects zero limits that could otherwise make lifecycle behavior ambiguous.
    pub const fn try_new(
        capacity: usize,
        lease_duration_millis: u64,
        max_body_bytes: usize,
        max_response_bytes: usize,
    ) -> Result<Self, ConfigError> {
        if capacity == 0 {
            return Err(ConfigError::ZeroCapacity);
        }
        if lease_duration_millis == 0 {
            return Err(ConfigError::ZeroLeaseDuration);
        }
        if max_body_bytes == 0 {
            return Err(ConfigError::ZeroBodyLimit);
        }
        if max_response_bytes == 0 {
            return Err(ConfigError::ZeroResponseLimit);
        }
        Ok(Self::new(
            capacity,
            lease_duration_millis,
            max_body_bytes,
            max_response_bytes,
        ))
    }

    pub(crate) const fn capacity(self) -> usize {
        self.capacity
    }

    pub(crate) const fn lease_duration_millis(self) -> u64 {
        self.lease_duration_millis
    }

    pub(crate) const fn max_body_bytes(self) -> usize {
        self.max_body_bytes
    }

    pub(crate) const fn max_response_bytes(self) -> usize {
        self.max_response_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    ZeroCapacity,
    ZeroLeaseDuration,
    ZeroBodyLimit,
    ZeroResponseLimit,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::ZeroCapacity => "capacity must be positive",
            Self::ZeroLeaseDuration => "lease duration must be positive",
            Self::ZeroBodyLimit => "body limit must be positive",
            Self::ZeroResponseLimit => "response limit must be positive",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for ConfigError {}
