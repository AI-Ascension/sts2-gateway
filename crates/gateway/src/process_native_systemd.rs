// SPDX-License-Identifier: MIT

use super::command;
use super::configuration::{NativeConfiguration, NativeProfile};
use super::{
    GenerationRecord, NativeProcessError, ProcessFault, ServiceController, StopMode,
    UnitObservation,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[path = "process_native_systemd_arguments.rs"]
mod arguments;
pub(super) use arguments::launch_arguments;
#[path = "process_native_systemd_properties.rs"]
pub(super) mod properties;
use properties::{description, number, terminal};

const PROPERTIES: &str = "LoadState,Description,Transient,InvocationID,ControlGroup,MainPID,ExecMainPID,KillMode,SendSIGKILL,Restart,Delegate,ProtectControlGroups,RemainAfterExit,ExitType,ActiveState,SubState,Job";

pub(super) struct SystemdController {
    boot: String,
    systemctl: PathBuf,
    systemd_run: PathBuf,
    runtime_directory: PathBuf,
    scratch: PathBuf,
}

impl SystemdController {
    pub(super) fn open(configuration: &NativeConfiguration) -> Result<Self, NativeProcessError> {
        let controller = Self {
            boot: super::identity::boot_id()?,
            systemctl: configuration.systemctl_path.clone(),
            systemd_run: configuration.systemd_run_path.clone(),
            runtime_directory: configuration.runtime_directory.clone(),
            scratch: configuration
                .registry_path
                .parent()
                .ok_or(NativeProcessError::Configuration)?
                .to_path_buf(),
        };
        for executable in [&controller.systemctl, &controller.systemd_run] {
            let output = command::run(
                executable,
                &["--version".to_owned()],
                &controller.runtime_directory,
                &controller.scratch,
                Duration::from_secs(2),
            )?;
            command::version(&output)?;
        }
        let output = controller.control(
            &["show", "--property=Version", "--value"],
            Duration::from_secs(2),
        )?;
        command::version(&output)?;
        Ok(controller)
    }

    fn control(
        &self,
        arguments: &[&str],
        budget: Duration,
    ) -> Result<command::CommandOutput, NativeProcessError> {
        let mut args = vec![
            "--user".to_owned(),
            "--no-pager".to_owned(),
            "--no-ask-password".to_owned(),
        ];
        args.extend(arguments.iter().map(|value| (*value).to_owned()));
        command::run(
            &self.systemctl,
            &args,
            &self.runtime_directory,
            &self.scratch,
            budget,
        )
    }

    fn verified_properties(
        &self,
        record: &GenerationRecord,
        budget: Duration,
    ) -> Result<Option<BTreeMap<String, String>>, ProcessFault> {
        let output = self
            .control(
                &["show", &record.unit, &format!("--property={PROPERTIES}")],
                budget,
            )
            .map_err(|_| ProcessFault::Unavailable)?;
        let mut properties = properties::parse(&output.bytes)?;
        if properties.get("LoadState").map(String::as_str) == Some("not-found") {
            return Ok(None);
        }
        if !output.success {
            return Err(ProcessFault::InspectionFailed);
        }
        properties::verify(record, &mut properties, super::identity::empty_group)?;
        Ok(Some(properties))
    }
}

impl ServiceController for SystemdController {
    fn boot_id(&self) -> &str {
        &self.boot
    }

    fn start(
        &mut self,
        record: &GenerationRecord,
        profile: &NativeProfile,
    ) -> Result<(), ProcessFault> {
        let arguments = launch_arguments(record, profile);
        let output = command::run(
            &self.systemd_run,
            &arguments,
            &self.runtime_directory,
            &self.scratch,
            Duration::from_millis(profile.profile.policy().start_timeout_millis()),
        )
        .map_err(|_| ProcessFault::Unavailable)?;
        if !output.success {
            return Err(ProcessFault::Unavailable);
        }
        Ok(())
    }

    fn observe(
        &mut self,
        record: &GenerationRecord,
        profile: &NativeProfile,
    ) -> Result<UnitObservation, ProcessFault> {
        if record.boot_id != self.boot
            || super::identity::boot_id().map_err(|_| ProcessFault::Unavailable)? != self.boot
        {
            return Err(ProcessFault::IdentityMismatch);
        }
        let Some(properties) = self.verified_properties(record, Duration::from_secs(2))? else {
            return Ok(UnitObservation::Missing);
        };
        let invocation = properties
            .get("InvocationID")
            .cloned()
            .ok_or(ProcessFault::InspectionFailed)?;
        let control_group = properties
            .get("ControlGroup")
            .cloned()
            .ok_or(ProcessFault::InspectionFailed)?;
        let main_pid = number(&properties, "MainPID")?;
        let main_pid = if main_pid == 0 {
            number(&properties, "ExecMainPID")?
        } else {
            main_pid
        };
        let members = super::identity::group_members(
            &control_group,
            profile.profile.policy().max_descendants() + 1,
        )?;
        let terminal = terminal(&properties);
        Ok(UnitObservation::Present {
            invocation,
            control_group,
            main_pid,
            members,
            terminal,
        })
    }

    fn stop(
        &mut self,
        record: &GenerationRecord,
        profile: &NativeProfile,
        mode: StopMode,
    ) -> Result<(), ProcessFault> {
        let deadline =
            Instant::now() + Duration::from_millis(profile.profile.policy().stop_timeout_millis());
        if self
            .verified_properties(record, deadline.saturating_duration_since(Instant::now()))?
            .is_none()
        {
            return Err(ProcessFault::InspectionFailed);
        }
        if mode == StopMode::Force {
            let output = self
                .control(
                    &["kill", "--kill-whom=all", "--signal=SIGKILL", &record.unit],
                    deadline.saturating_duration_since(Instant::now()),
                )
                .map_err(|_| ProcessFault::StopTimedOut)?;
            if !output.success {
                return Err(ProcessFault::StopFailed);
            }
        }
        let output = self
            .control(
                &["stop", &record.unit],
                deadline.saturating_duration_since(Instant::now()),
            )
            .map_err(|_| ProcessFault::StopTimedOut)?;
        if !output.success {
            return Err(ProcessFault::StopFailed);
        }
        Ok(())
    }

    fn empty_group(&self, path: &str) -> Result<bool, ProcessFault> {
        super::identity::empty_group(path)
    }
}
