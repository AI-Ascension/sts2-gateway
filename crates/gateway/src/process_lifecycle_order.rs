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
