// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::ledger_types::SaveProfileStatus;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryGuidance {
    pub code: String,
    pub action: String,
}

impl RecoveryGuidance {
    pub fn for_status(status: SaveProfileStatus) -> Option<Self> {
        match status {
            SaveProfileStatus::Unknown => Some(Self {
                code: String::from("save_profile_reconcile_required"),
                action: String::from("lookup the original operation before retrying"),
            }),
            SaveProfileStatus::Blocked => Some(Self {
                code: String::from("save_profile_operator_intervention_required"),
                action: String::from(
                    "inspect the isolated allocation and clear the reported block",
                ),
            }),
            _ => None,
        }
    }
}
