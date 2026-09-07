// SPDX-License-Identifier: MIT

use serde_json::json;

use super::super::recovery_frame::{RecoveryFrame, RecoveryKind, response_frame, response_result};
use super::{RuntimeService, json_error};

impl RuntimeService {
    pub(super) fn recovery_lease_revoke(&mut self, frame: &RecoveryFrame) -> (u16, Vec<u8>) {
        let Some(proof) = self.lease_proof_from_wire(&frame.payload()["lease"]) else {
            return (409, json_error("recovery_stale_lease"));
        };
        let Some(reason) = frame.payload()["reason"].as_str() else {
            return (400, json_error("recovery_revoke_reason_invalid"));
        };
        if !matches!(
            reason,
            "operator" | "shutdown" | "incarnation_replaced" | "suspend_ambiguous" | "rekey"
        ) {
            return (400, json_error("recovery_revoke_reason_invalid"));
        }
        let Some(store) = self.recovery.as_mut() else {
            return (503, json_error("recovery_persistence_unavailable"));
        };
        if let Err(error) = store.revoke_lease(&proof, reason) {
            return super::recovery_wire::recovery_store_error(error);
        }
        self.recovery_lease = None;
        self.recovery_lease_deadline = None;
        self.lease_active = false;
        self.lease_revoked = true;
        let body = json!({ "result": response_result("LEASE_REVOKED", false, None) });
        (
            200,
            response_frame(
                RecoveryKind::LeaseRevoke,
                frame.correlation(),
                &self.config.caller_id,
                body,
            ),
        )
    }
}
