// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use crate::process_profile::LaunchProfileId;
use crate::{InstanceId, OperationId, ProcessIdentity};

/// Durable authoritative ownership for one instance.
///
/// The ownership row is separate from request history. Its presence reserves
/// capacity even when `process` is `None` and the latest lifecycle operation is
/// still being recovered.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleOwnership {
    instance_id: InstanceId,
    operation_id: OperationId,
    sequence: u64,
    profile_id: LaunchProfileId,
    process: Option<ProcessIdentity>,
}

impl LifecycleOwnership {
    pub(crate) fn new(
        instance_id: InstanceId,
        operation_id: OperationId,
        sequence: u64,
        profile_id: LaunchProfileId,
        process: Option<ProcessIdentity>,
    ) -> Self {
        Self {
            instance_id,
            operation_id,
            sequence,
            profile_id,
            process,
        }
    }

    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn profile_id(&self) -> LaunchProfileId {
        self.profile_id
    }

    pub const fn process(&self) -> Option<&ProcessIdentity> {
        self.process.as_ref()
    }
}
