// SPDX-License-Identifier: MIT

//! Durable gateway-side state for a matching host lease installation.

use serde::{Deserialize, Serialize};

use super::super::recovery_validation::RecoveryStoreError;

/// The plaintext fence token is intentionally absent: only the in-memory
/// lease returned by an issuance operation may carry it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryHostLeaseState {
    Uninstalled,
    PendingHostInstall,
    Installed,
    PendingHostRenew,
    PendingHostRevoke,
    HostRevoked,
    RestartInvalidated,
}

impl RecoveryHostLeaseState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Uninstalled => "UNINSTALLED",
            Self::PendingHostInstall => "PENDING_HOST_INSTALL",
            Self::Installed => "INSTALLED",
            Self::PendingHostRenew => "PENDING_HOST_RENEW",
            Self::PendingHostRevoke => "PENDING_HOST_REVOKE",
            Self::HostRevoked => "HOST_REVOKED",
            Self::RestartInvalidated => "RESTART_INVALIDATED",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, RecoveryStoreError> {
        match value {
            "UNINSTALLED" => Ok(Self::Uninstalled),
            "PENDING_HOST_INSTALL" => Ok(Self::PendingHostInstall),
            "INSTALLED" => Ok(Self::Installed),
            "PENDING_HOST_RENEW" => Ok(Self::PendingHostRenew),
            "PENDING_HOST_REVOKE" => Ok(Self::PendingHostRevoke),
            "HOST_REVOKED" => Ok(Self::HostRevoked),
            "RESTART_INVALIDATED" => Ok(Self::RestartInvalidated),
            _ => Err(RecoveryStoreError::Corrupt(
                "unknown host lease state".to_owned(),
            )),
        }
    }

    pub(crate) const fn is_mutation_ready(self) -> bool {
        matches!(self, Self::Installed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryHostLeaseBinding {
    pub lease_id: String,
    pub installation_id: Option<String>,
    pub grant_digest: Option<String>,
    pub state: RecoveryHostLeaseState,
    pub host_install_generation: u64,
    pub host_renew_sequence: u64,
    pub host_ack_message_id: Option<String>,
    pub host_ack_recorded_at_millis: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryLeaseProof {
    pub deployment_id: String,
    pub instance_id: String,
    pub instance_incarnation: String,
    pub boot_id: String,
    pub authority_generation: u64,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub fence_token: String,
}

impl super::RecoveryLease {
    pub fn proof(&self) -> RecoveryLeaseProof {
        RecoveryLeaseProof {
            deployment_id: self.deployment_id.clone(),
            instance_id: self.instance_id.clone(),
            instance_incarnation: self.instance_incarnation.clone(),
            boot_id: self.boot_id.clone(),
            authority_generation: self.authority_generation,
            lease_id: self.lease_id.clone(),
            lease_epoch: self.lease_epoch,
            fence_token: self.fence_token.clone(),
        }
    }
}
