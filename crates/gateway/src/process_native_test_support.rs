// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct State {
    pub(super) starts: usize,
    pub(super) stops: usize,
    pub(super) lost_ack: bool,
    pub(super) survive_stop: bool,
    pub(super) groups_empty: bool,
    pub(super) units: BTreeMap<String, UnitObservation>,
}

#[derive(Clone, Default)]
pub(super) struct Controller(pub(super) Arc<Mutex<State>>);
impl ServiceController for Controller {
    fn boot_id(&self) -> &str {
        "00000000-0000-0000-0000-000000000001"
    }
    fn start(
        &mut self,
        record: &GenerationRecord,
        profile: &configuration::NativeProfile,
    ) -> Result<(), ProcessFault> {
        let mut state = self.0.lock().expect("fixture lock");
        state.starts += 1;
        let pid = 100 + state.starts as u64;
        state.units.insert(
            record.unit.clone(),
            UnitObservation::Present {
                invocation: uuid::Uuid::new_v4().simple().to_string(),
                control_group: format!("/user.slice/test.scope/{}", record.unit),
                main_pid: pid,
                members: vec![NativeMember {
                    pid,
                    birth: 900 + pid,
                    control_group: format!("/user.slice/test.scope/{}", record.unit),
                    executable: profile.executable.clone(),
                    image_digest: profile.image_digest.clone(),
                }],
                terminal: false,
            },
        );
        if state.lost_ack {
            return Err(ProcessFault::Unavailable);
        }
        Ok(())
    }
    fn observe(
        &mut self,
        record: &GenerationRecord,
        _: &configuration::NativeProfile,
    ) -> Result<UnitObservation, ProcessFault> {
        Ok(self
            .0
            .lock()
            .expect("fixture lock")
            .units
            .get(&record.unit)
            .cloned()
            .unwrap_or(UnitObservation::Missing))
    }
    fn stop(
        &mut self,
        record: &GenerationRecord,
        _: &configuration::NativeProfile,
        _: StopMode,
    ) -> Result<(), ProcessFault> {
        let mut state = self.0.lock().expect("fixture lock");
        state.stops += 1;
        if !state.survive_stop
            && let Some(UnitObservation::Present {
                members, terminal, ..
            }) = state.units.get_mut(&record.unit)
        {
            members.clear();
            *terminal = true;
        }
        Ok(())
    }
    fn empty_group(&self, path: &str) -> Result<bool, ProcessFault> {
        let state = self.0.lock().expect("fixture lock");
        for observation in state.units.values() {
            if let UnitObservation::Present {
                control_group,
                members,
                ..
            } = observation
                && control_group == path
            {
                return Ok(members.is_empty());
            }
        }
        Ok(state.groups_empty)
    }
}

pub(super) struct Directory(pub(super) PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct Fixture {
    pub(super) port: port::NativeProcessPort<Controller>,
    pub(super) controller: Controller,
    pub(super) profiles: BTreeMap<u64, configuration::NativeProfile>,
    pub(super) root: Directory,
}
impl Fixture {
    pub(super) fn new(capacity: usize) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/process-native-tests")
            .join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(root.join("user-data")).expect("fixture roots");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&root, &root.join("user-data")] {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                    .expect("private root");
            }
        }
        let root = root.canonicalize().expect("canonical fixture root");
        let executable = root.join("approved-image");
        std::fs::write(&executable, b"synthetic executable image; never executed")
            .expect("fixture image");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                .expect("executable fixture");
        }
        let image_digest = configuration::image_digest(&executable).expect("image digest");
        let profile = profile();
        let binding = configuration::NativeProfile {
            profile,
            executable,
            image_digest,
            user_data: root.join("user-data"),
            arguments: vec!["--fixture".to_owned()],
            digest: configuration::hex_digest(b"fixture configuration"),
        };
        let profiles = BTreeMap::from([(profile.id().value(), binding)]);
        let controller = Controller::default();
        let registry = store::GenerationRegistry::open(&root.join("registry.sqlite"), capacity)
            .expect("registry");
        Self {
            port: port::NativeProcessPort::new(profiles.clone(), controller.clone(), registry),
            controller,
            profiles,
            root: Directory(root),
        }
    }
    pub(super) fn start(&mut self, operation: u64) -> ProcessIdentity {
        self.port
            .start_generation(
                &generation(operation, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile(),
            )
            .expect("fixture launch")
            .identity()
            .clone()
    }
}

pub(super) fn profile() -> LaunchProfile {
    LaunchProfile::try_new(
        crate::LaunchProfileId::new(1),
        crate::ExecutableIdentity::new(11, 22, 33),
        crate::UserDataConfig::new(44),
        crate::ProcessPolicy::try_new(4, 100, 100).expect("policy"),
    )
    .expect("profile")
}

pub(super) fn generation(operation: u64, restart: bool) -> ProcessOperationGeneration {
    use crate::{
        AuthorityEpoch, CallerId, LeaseEpoch, LeaseId, LeaseProof, LifecycleAction,
        LifecycleOperation, LifecycleOperationState, OperationId, ProcessGenerationKind, SessionId,
    };
    let instance = InstanceId::new(7);
    let lease = LeaseProof::new(
        instance,
        CallerId::new(2),
        SessionId::new(3),
        LeaseId::new(4),
        LeaseEpoch::new(1),
    );
    let action = if restart {
        LifecycleAction::Restart {
            profile_id: profile().id(),
        }
    } else {
        LifecycleAction::LaunchNew {
            profile_id: profile().id(),
        }
    };
    let mut record = LifecycleOperation::new(
        OperationId::new(operation),
        instance,
        lease,
        AuthorityEpoch::new(1),
        action,
    );
    record.set_sequence(operation);
    record.set_authority_epoch(AuthorityEpoch::new(if restart { 2 } else { 1 }));
    record.set_state(
        if restart {
            LifecycleOperationState::Restarting
        } else {
            LifecycleOperationState::Starting
        },
        None,
        None,
    );
    ProcessOperationGeneration::for_operation(
        &record,
        profile(),
        if restart {
            ProcessGenerationKind::RestartReplacement
        } else {
            ProcessGenerationKind::LaunchNew
        },
    )
    .expect("persisted generation")
}
