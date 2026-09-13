// SPDX-License-Identifier: MIT

use crate::{InstanceId, LaunchProfileId, LifecycleError};

use super::ProcessLifecycle;

impl<C, P, S, F> ProcessLifecycle<C, P, S, F>
where
    C: crate::Clock,
    P: crate::ProcessPort,
    S: crate::LifecycleRecordStore,
    F: crate::LeaseDecisionPort,
{
    /// Ensures that a profile's server-owned user-data namespace is not
    /// concurrently used by another active instance. A missing/stale profile
    /// in an active durable record is treated as a conflict: the coordinator
    /// cannot prove that the namespace is isolated, so it must fail closed
    /// before invoking the process adapter.
    pub(crate) fn ensure_profile_namespace_available(
        &self,
        instance_id: InstanceId,
        profile_id: LaunchProfileId,
    ) -> Result<(), LifecycleError> {
        let profile = self.profiles.resolve(profile_id)?;
        let namespace = profile.user_data();

        let ownership_conflict = self.ownership.values().any(|owner| {
            if owner.instance_id() == instance_id {
                return false;
            }
            match self.profiles.resolve(owner.profile_id()) {
                Ok(existing) => existing.user_data() == namespace,
                Err(_) => true,
            }
        });
        if ownership_conflict {
            return Err(LifecycleError::UserDataNamespaceBusy);
        }

        let active_record_conflict = self.records.values().any(|operation| {
            if operation.instance_id() == instance_id || !operation.state().is_active() {
                return false;
            }
            let Some(existing_id) = self.profile_id_for_record(operation) else {
                // An active operation without a resolvable profile still
                // represents an unknown reservation. Do not launch into a
                // namespace that might be shared with it.
                return true;
            };
            match self.profiles.resolve(existing_id) {
                Ok(existing) => existing.user_data() == namespace,
                Err(_) => true,
            }
        });
        if active_record_conflict {
            return Err(LifecycleError::UserDataNamespaceBusy);
        }

        Ok(())
    }
}
