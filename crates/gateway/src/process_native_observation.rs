// SPDX-License-Identifier: MIT

use super::*;

impl<C: ServiceController> NativeProcessPort<C> {
    pub(super) fn observe(
        &mut self,
        record: &mut GenerationRecord,
        instance: InstanceId,
    ) -> Result<Vec<NativeMember>, ProcessFault> {
        let profile = self.checked_record(record)?;
        if record
            .identity
            .as_ref()
            .is_some_and(|identity| identity.instance_id() != instance)
        {
            return Err(ProcessFault::IdentityMismatch);
        }
        if !record.dispatched {
            return Ok(Vec::new());
        }
        let observation = self.controller.observe(record, &profile)?;
        if matches!(observation, UnitObservation::Present { .. }) {
            self.bind_observation(record, instance, &profile, &observation)?;
        }
        match observation {
            UnitObservation::Missing => {
                if let Some(group) = &record.control_group
                    && self.controller.empty_group(group)?
                {
                    record.absent = true;
                    self.registry
                        .save(record)
                        .map_err(|_| ProcessFault::Unavailable)?;
                    return Ok(Vec::new());
                }
                Err(ProcessFault::InspectionFailed)
            }
            UnitObservation::Present {
                members, terminal, ..
            } => {
                if record.absent && !members.is_empty() {
                    return Err(ProcessFault::IdentityMismatch);
                }
                if members.is_empty()
                    && terminal
                    && self.controller.empty_group(
                        record
                            .control_group
                            .as_deref()
                            .ok_or(ProcessFault::InspectionFailed)?,
                    )?
                {
                    record.absent = true;
                }
                self.registry
                    .save(record)
                    .map_err(|_| ProcessFault::Unavailable)?;
                Ok(members)
            }
        }
    }

    fn bind_observation(
        &self,
        record: &mut GenerationRecord,
        instance: InstanceId,
        profile: &NativeProfile,
        observation: &UnitObservation,
    ) -> Result<(), ProcessFault> {
        let UnitObservation::Present {
            invocation,
            control_group,
            main_pid,
            members,
            ..
        } = observation
        else {
            return Err(ProcessFault::InspectionFailed);
        };
        if invocation.len() != 32
            || uuid::Uuid::parse_str(invocation).is_err()
            || invocation == "00000000000000000000000000000000"
            || !super::super::identity::valid_group(control_group, &record.unit)
            || record
                .invocation
                .as_ref()
                .is_some_and(|value| value != invocation)
            || record
                .control_group
                .as_ref()
                .is_some_and(|value| value != control_group)
            || members.len() > profile.profile.policy().max_descendants() + 1
        {
            return Err(ProcessFault::IdentityMismatch);
        }
        for member in members {
            if member.pid == 0
                || member.birth == 0
                || member.control_group != *control_group
                || member.image_digest != profile.image_digest
                || member.executable != profile.executable
            {
                return Err(ProcessFault::IdentityMismatch);
            }
        }
        if let Some(identity) = &record.identity {
            if *main_pid != 0 && *main_pid != identity.pid() {
                return Err(ProcessFault::IdentityMismatch);
            }
            if members
                .iter()
                .any(|member| member.pid == identity.pid() && member.birth != identity.birth_id())
            {
                return Err(ProcessFault::IdentityMismatch);
            }
        } else if let Some(main) = members.iter().find(|member| member.pid == *main_pid) {
            record.identity = Some(ProcessIdentity::new(
                instance,
                ProcessHandle::new(record.handle),
                main.pid,
                main.birth,
                profile.profile.executable(),
                profile.profile.user_data(),
            ));
        }
        record.invocation = Some(invocation.clone());
        record.control_group = Some(control_group.clone());
        Ok(())
    }
}
