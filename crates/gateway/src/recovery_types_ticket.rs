// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::RecoveryStoreError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecoveryTicketState {
    Issued,
    Admitted,
    Executing,
    EffectWitnessRecorded,
    Settled,
    Rejected,
    Unknown,
}

impl RecoveryTicketState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Issued => "ISSUED",
            Self::Admitted => "ADMITTED",
            Self::Executing => "EXECUTING",
            Self::EffectWitnessRecorded => "EFFECT_WITNESS_RECORDED",
            Self::Settled => "SETTLED",
            Self::Rejected => "REJECTED",
            Self::Unknown => "UNKNOWN",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, RecoveryStoreError> {
        match value {
            "ISSUED" => Ok(Self::Issued),
            "ADMITTED" => Ok(Self::Admitted),
            "EXECUTING" => Ok(Self::Executing),
            "EFFECT_WITNESS_RECORDED" => Ok(Self::EffectWitnessRecorded),
            "SETTLED" => Ok(Self::Settled),
            "REJECTED" => Ok(Self::Rejected),
            "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(RecoveryStoreError::Corrupt(
                "unknown admission ticket state".to_owned(),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryAdmissionTicket {
    pub ticket_id: String,
    pub operation_id: String,
    pub payload_digest: String,
    pub boot_id: String,
    pub instance_incarnation: String,
    pub lease_epoch: u64,
    pub host_fence_id: String,
    pub state: RecoveryTicketState,
    pub issued_at_millis: u64,
    pub expires_at_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryStoreConfig {
    pub unresolved_capacity: usize,
    pub retained_capacity: usize,
    pub lease_ttl_seconds: u64,
    pub lease_renewal_interval_seconds: u64,
}

impl Default for RecoveryStoreConfig {
    fn default() -> Self {
        Self {
            unresolved_capacity: 64,
            retained_capacity: 4_096,
            lease_ttl_seconds: 30,
            lease_renewal_interval_seconds: 10,
        }
    }
}

impl RecoveryStoreConfig {
    pub fn validate(&self) -> Result<(), RecoveryStoreError> {
        if self.unresolved_capacity == 0
            || self.retained_capacity < self.unresolved_capacity
            || !(5..=300).contains(&self.lease_ttl_seconds)
            || self.lease_renewal_interval_seconds == 0
            || self.lease_renewal_interval_seconds >= self.lease_ttl_seconds
        {
            return Err(RecoveryStoreError::InvalidInput(
                "invalid recovery capacity or lease policy".to_owned(),
            ));
        }
        Ok(())
    }
}
