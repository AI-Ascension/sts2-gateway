// SPDX-License-Identifier: MIT

use crate::process_store::{LifecycleOperation, LifecycleOwnership};

pub(crate) fn operation_order(operation: &LifecycleOperation) -> (u8, u64, u64) {
    if operation.sequence() == 0 {
        (0, operation.operation_id().value(), 0)
    } else {
        (1, operation.sequence(), operation.operation_id().value())
    }
}

pub(crate) fn ownership_sequence(operation: &LifecycleOperation) -> u64 {
    // Keep zero as the legacy marker; `operation_order` supplies the
    // operation-ID fallback only while comparing legacy records.
    operation.sequence()
}

pub(crate) fn owner_order(owner: &LifecycleOwnership) -> (u8, u64, u64) {
    if owner.sequence() == 0 {
        (0, owner.operation_id().value(), 0)
    } else {
        (1, owner.sequence(), owner.operation_id().value())
    }
}

impl<C, P, S, F> super::ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    pub(crate) fn lease_matches(&self, operation: &LifecycleOperation) -> bool {
        self.leases
            .get(&operation.instance_id())
            .is_some_and(|lease| lease.proof() == operation.lease())
    }

    pub(crate) fn operation_can_update_owner(&self, operation: &LifecycleOperation) -> bool {
        self.ownership
            .get(&operation.instance_id())
            .is_none_or(|owner| operation_order(operation) >= owner_order(owner))
    }
}
