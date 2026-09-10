// SPDX-License-Identifier: MIT

/// The owner-issued identity that authorizes one Runtime-v2 gateway context.
///
/// `boot_epoch` is intentionally outside the frozen Runtime-v2 envelope. It is an owner-boundary
/// proof that must be carried by the workflow coordinator alongside the protocol message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2Authority {
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    boot_epoch: String,
}

impl RuntimeV2Authority {
    /// Creates a validated owner-bound authority identity.
    pub fn new(
        instance_id: &str,
        session_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        boot_epoch: &str,
    ) -> Result<Self, RuntimeV2ValidationError> {
        for identity in [instance_id, session_id, lease_id, boot_epoch] {
            validate_identity(identity)?;
        }
        if lease_epoch > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2ValidationError::GenerationBounds);
        }
        Ok(Self {
            instance_id: instance_id.to_owned(),
            session_id: session_id.to_owned(),
            lease_id: lease_id.to_owned(),
            lease_epoch,
            boot_epoch: boot_epoch.to_owned(),
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    pub fn boot_epoch(&self) -> &str {
        &self.boot_epoch
    }

    fn validate_against(&self, current: &Self) -> Result<(), RuntimeV2RecoveryError> {
        if self.instance_id != current.instance_id
            || self.session_id != current.session_id
            || self.lease_id != current.lease_id
            || self.lease_epoch != current.lease_epoch
        {
            return Err(RuntimeV2RecoveryError::AuthorityMismatch);
        }
        if self.boot_epoch != current.boot_epoch {
            return Err(RuntimeV2RecoveryError::StaleBootEpoch);
        }
        Ok(())
    }
}

/// Failure domains are independent recovery claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2RecoveryFailureDomain {
    HarnessRestart,
    GatewayRestart,
    HostRestart,
    MachineReboot,
}

impl fmt::Display for RuntimeV2RecoveryFailureDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::HarnessRestart => "harness restart",
            Self::GatewayRestart => "gateway restart",
            Self::HostRestart => "host restart",
            Self::MachineReboot => "machine reboot",
        };
        formatter.write_str(text)
    }
}

/// The scope and lifetime of receipts available to recovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2ReceiptRetention {
    Unavailable,
    ProcessLifetime { max_operations: usize },
    Durable {
        max_operations: usize,
        retention_seconds: u64,
    },
}

impl RuntimeV2ReceiptRetention {
    pub const fn unavailable() -> Self {
        Self::Unavailable
    }

    pub const fn process_lifetime(max_operations: usize) -> Self {
        Self::ProcessLifetime { max_operations }
    }

    pub const fn durable(max_operations: usize, retention_seconds: u64) -> Self {
        Self::Durable {
            max_operations,
            retention_seconds,
        }
    }

    pub const fn is_available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    fn validate(self) -> Result<(), RuntimeV2RecoveryError> {
        match self {
            Self::Unavailable => Ok(()),
            Self::ProcessLifetime { max_operations }
            | Self::Durable {
                max_operations,
                ..
            } if max_operations == 0 => Err(RuntimeV2RecoveryError::InvalidReceiptRetention),
            Self::ProcessLifetime { .. } => Ok(()),
            Self::Durable {
                retention_seconds: 0,
                ..
            } => Err(RuntimeV2RecoveryError::InvalidReceiptRetention),
            Self::Durable { .. } => Ok(()),
        }
    }
}

/// Explicit recovery support advertised by the gateway owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeV2RecoveryCapabilities {
    harness_restart: bool,
    gateway_restart: bool,
    host_restart: bool,
    machine_reboot: bool,
    receipt_retention: RuntimeV2ReceiptRetention,
}

impl RuntimeV2RecoveryCapabilities {
    #[must_use]
    pub const fn new(
        harness_restart: bool,
        gateway_restart: bool,
        host_restart: bool,
        machine_reboot: bool,
        receipt_retention: RuntimeV2ReceiptRetention,
    ) -> Self {
        Self {
            harness_restart,
            gateway_restart,
            host_restart,
            machine_reboot,
            receipt_retention,
        }
    }

    #[must_use]
    pub const fn unsupported() -> Self {
        Self::new(
            false,
            false,
            false,
            false,
            RuntimeV2ReceiptRetention::Unavailable,
        )
    }

    pub const fn supports(self, domain: RuntimeV2RecoveryFailureDomain) -> bool {
        match domain {
            RuntimeV2RecoveryFailureDomain::HarnessRestart => self.harness_restart,
            RuntimeV2RecoveryFailureDomain::GatewayRestart => self.gateway_restart,
            RuntimeV2RecoveryFailureDomain::HostRestart => self.host_restart,
            RuntimeV2RecoveryFailureDomain::MachineReboot => self.machine_reboot,
        }
    }

    pub fn require(
        self,
        domain: RuntimeV2RecoveryFailureDomain,
    ) -> Result<(), RuntimeV2RecoveryError> {
        if self.supports(domain) {
            Ok(())
        } else {
            Err(RuntimeV2RecoveryError::UnsupportedFailureDomain(domain))
        }
    }

    pub const fn harness_restart(self) -> bool {
        self.harness_restart
    }

    pub const fn gateway_restart(self) -> bool {
        self.gateway_restart
    }

    pub const fn host_restart(self) -> bool {
        self.host_restart
    }

    pub const fn machine_reboot(self) -> bool {
        self.machine_reboot
    }

    pub const fn receipt_retention(self) -> RuntimeV2ReceiptRetention {
        self.receipt_retention
    }
}

/// Recovery admission contract for workflow-facing Runtime-v2 use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2RecoveryContract {
    authority: RuntimeV2Authority,
    capabilities: RuntimeV2RecoveryCapabilities,
}

impl RuntimeV2RecoveryContract {
    pub fn new(
        authority: RuntimeV2Authority,
        capabilities: RuntimeV2RecoveryCapabilities,
    ) -> Result<Self, RuntimeV2RecoveryError> {
        capabilities.receipt_retention.validate()?;
        Ok(Self {
            authority,
            capabilities,
        })
    }

    pub fn authority(&self) -> &RuntimeV2Authority {
        &self.authority
    }

    pub const fn capabilities(&self) -> RuntimeV2RecoveryCapabilities {
        self.capabilities
    }

    pub fn require_domain(
        &self,
        domain: RuntimeV2RecoveryFailureDomain,
    ) -> Result<(), RuntimeV2RecoveryError> {
        self.capabilities.require(domain)
    }

    pub fn require_receipt_access(&self) -> Result<(), RuntimeV2RecoveryError> {
        if self.capabilities.receipt_retention.is_available() {
            Ok(())
        } else {
            Err(RuntimeV2RecoveryError::ReceiptRetentionUnavailable)
        }
    }

    fn validate_authority(&self, presented: &RuntimeV2Authority) -> Result<(), RuntimeV2RecoveryError> {
        presented.validate_against(&self.authority)
    }
}

/// Fail-closed recovery admission failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2RecoveryError {
    AuthorityRequired,
    AuthorityMismatch,
    StaleBootEpoch,
    UnsupportedFailureDomain(RuntimeV2RecoveryFailureDomain),
    ReceiptRetentionUnavailable,
    InvalidReceiptRetention,
}

impl fmt::Display for RuntimeV2RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthorityRequired => formatter.write_str("workflow authority proof is required"),
            Self::AuthorityMismatch => formatter.write_str("workflow authority identity mismatched"),
            Self::StaleBootEpoch => formatter.write_str("workflow boot epoch is stale"),
            Self::UnsupportedFailureDomain(domain) => {
                write!(formatter, "recovery is unsupported after {domain}")
            }
            Self::ReceiptRetentionUnavailable => {
                formatter.write_str("retained receipt recovery is unavailable")
            }
            Self::InvalidReceiptRetention => {
                formatter.write_str("receipt retention bounds are invalid")
            }
        }
    }
}

impl std::error::Error for RuntimeV2RecoveryError {}
