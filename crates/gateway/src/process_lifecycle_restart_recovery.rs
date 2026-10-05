// SPDX-License-Identifier: MIT

use crate::process_store::LifecycleOperation;
use crate::{LaunchProfile, LifecycleError};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn prepare_restart_recovery(
        &mut self,
        operation: LifecycleOperation,
        profile: LaunchProfile,
    ) -> Result<LifecycleOperation, LifecycleError> {
        let identity = operation
            .process()
            .cloned()
            .ok_or(LifecycleError::IdentityMismatch)?;
        if operation.request_epoch() == operation.authority_epoch() {
            // Older persisted records may carry Restarting and the prior
            // identity before the authority-rotation update became durable.
            return self.begin_restart(operation, identity);
        }
        if operation.authority_epoch() != self.current_authority_epoch(operation.instance_id()) {
            return Err(LifecycleError::StaleAuthorityEpoch);
        }
        // A prior rotation may have committed while ownership reservation
        // failed. Retry that reservation before effects without rotating twice.
        self.reserve_ownership(&operation, profile.id())?;
        Ok(operation)
    }
}
