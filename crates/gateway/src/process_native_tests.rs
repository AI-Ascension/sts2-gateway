// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
#[path = "process_native_test_support.rs"]
mod support;
use support::*;
#[path = "process_native_deployment_tests.rs"]
mod deployment_tests;
#[path = "process_native_fault_tests.rs"]
mod fault_tests;

#[test]
fn duplicate_and_reopened_generation_only_observe_the_original_service() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    assert_eq!(fixture.start(1), identity);
    let Fixture {
        port,
        controller,
        profiles,
        root,
    } = fixture;
    drop(port);
    let registry =
        store::GenerationRegistry::open(&root.0.join("registry.sqlite"), 8).expect("reopen");
    let mut reopened = port::NativeProcessPort::new(profiles, controller.clone(), registry);
    assert_eq!(
        reopened
            .recover_generation(&generation(1, false), profile())
            .expect("recover"),
        GenerationRecovery::Found(identity.clone())
    );
    assert_eq!(
        reopened.inspect(identity.process()).expect("inspect"),
        ProcessState::Running
    );
    reopened
        .stop(identity.process(), StopMode::Graceful)
        .expect("owned stop");
    assert_eq!(
        reopened.inspect(identity.process()).expect("stopped"),
        ProcessState::Exited { code: None }
    );
    assert_eq!(controller.0.lock().expect("state").starts, 1);
    drop(reopened);
}

#[test]
fn lost_start_ack_reconciles_without_creating_another_process() {
    let mut fixture = Fixture::new(8);
    fixture.controller.0.lock().expect("state").lost_ack = true;
    assert!(
        fixture
            .port
            .start_generation(
                &generation(1, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile()
            )
            .is_err()
    );
    assert!(matches!(
        fixture
            .port
            .recover_generation(&generation(1, false), profile())
            .expect("recover"),
        GenerationRecovery::Found(_)
    ));
    fixture.start(1);
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 1);
}

#[test]
fn pre_dispatch_interruption_and_unknown_generations_never_authorize_a_start() {
    let mut fixture = Fixture::new(8);
    let binding = &fixture.profiles[&1];
    let key = store::generation_key(&generation(1, false));
    fixture
        .port
        .registry
        .reserve(
            &key,
            profile(),
            &binding.digest,
            fixture.controller.boot_id(),
        )
        .expect("persist undispatched intent");
    assert_eq!(
        fixture
            .port
            .recover_generation(&generation(1, false), profile())
            .expect("recovery"),
        GenerationRecovery::Indeterminate
    );
    assert!(
        fixture
            .port
            .start_generation(
                &generation(1, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile()
            )
            .is_err()
    );
    assert_eq!(
        fixture
            .port
            .recover_generation(&generation(2, false), profile())
            .expect("unknown"),
        GenerationRecovery::Indeterminate
    );
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 0);
}

#[test]
fn birth_image_install_cgroup_and_invocation_drift_refuse_stop_without_effects() {
    for corruption in 0..5 {
        let mut fixture = Fixture::new(8);
        let identity = fixture.start(1);
        {
            let mut state = fixture.controller.0.lock().expect("state");
            let UnitObservation::Present {
                invocation,
                members,
                ..
            } = state.units.values_mut().next().expect("unit")
            else {
                unreachable!()
            };
            match corruption {
                0 => *invocation = uuid::Uuid::new_v4().simple().to_string(),
                1 => members[0].birth += 1,
                2 => members[0].image_digest = "0".repeat(64),
                3 => members[0].executable = fixture.root.0.join("different-install"),
                _ => members[0].control_group = "/foreign.slice/control.service".to_owned(),
            }
        }
        assert_eq!(
            fixture.port.stop(identity.process(), StopMode::Force),
            Err(ProcessFault::IdentityMismatch)
        );
        assert_eq!(fixture.controller.0.lock().expect("state").stops, 0);
        assert!(
            fixture
                .port
                .stop(ProcessHandle::new(999), StopMode::Force)
                .is_err()
        );
    }
}

#[test]
fn surviving_descendants_prevent_false_stopped_and_are_recoverable() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    {
        let mut state = fixture.controller.0.lock().expect("state");
        state.survive_stop = true;
        let UnitObservation::Present { members, .. } =
            state.units.values_mut().next().expect("unit")
        else {
            unreachable!()
        };
        members.push(NativeMember {
            pid: 900,
            birth: 1900,
            control_group: members[0].control_group.clone(),
            executable: members[0].executable.clone(),
            image_digest: members[0].image_digest.clone(),
        });
    }
    assert_eq!(
        fixture.port.stop(identity.process(), StopMode::Force),
        Err(ProcessFault::StopTimedOut)
    );
    assert_eq!(
        fixture
            .port
            .descendants(identity.process())
            .expect("retained child")
            .len(),
        1
    );
    fixture.controller.0.lock().expect("state").survive_stop = false;
    fixture
        .port
        .stop(identity.process(), StopMode::Force)
        .expect("settled cleanup");
    assert!(
        fixture
            .port
            .descendants(identity.process())
            .expect("empty")
            .is_empty()
    );
}

#[test]
fn missing_unit_requires_retained_child_free_cgroup_evidence() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    fixture.controller.0.lock().expect("state").units.clear();
    assert_eq!(
        fixture.port.inspect(identity.process()),
        Err(ProcessFault::InspectionFailed)
    );
    fixture.controller.0.lock().expect("state").groups_empty = true;
    assert_eq!(
        fixture
            .port
            .inspect(identity.process())
            .expect("proven absence"),
        ProcessState::Exited { code: None }
    );
}

#[test]
fn empty_activating_unit_is_not_a_settled_stop() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    {
        let mut state = fixture.controller.0.lock().expect("state");
        state.survive_stop = true;
        let UnitObservation::Present {
            members, terminal, ..
        } = state.units.values_mut().next().expect("unit")
        else {
            unreachable!()
        };
        members.clear();
        *terminal = false;
    }
    assert_eq!(
        fixture.port.inspect(identity.process()),
        Err(ProcessFault::InspectionFailed)
    );
    assert_eq!(
        fixture.port.stop(identity.process(), StopMode::Force),
        Err(ProcessFault::StopTimedOut)
    );
}

#[test]
fn registry_capacity_and_profile_drift_fail_before_manager_effects() {
    let mut fixture = Fixture::new(1);
    let identity = fixture.start(1);
    fixture
        .port
        .stop(identity.process(), StopMode::Force)
        .expect("stop");
    assert!(
        fixture
            .port
            .start_generation(
                &generation(2, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile()
            )
            .is_err()
    );
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 1);
    let Fixture {
        port,
        controller,
        mut profiles,
        root,
    } = fixture;
    drop(port);
    profiles.get_mut(&1).expect("profile").digest = "0".repeat(64);
    let registry =
        store::GenerationRegistry::open(&root.0.join("registry.sqlite"), 1).expect("reopen");
    let mut reopened = port::NativeProcessPort::new(profiles, controller.clone(), registry);
    assert_eq!(
        reopened.recover_generation(&generation(1, false), profile()),
        Err(ProcessFault::IdentityMismatch)
    );
    assert_eq!(controller.0.lock().expect("state").starts, 1);
    drop(reopened);
}

#[test]
fn replacement_generation_is_distinct_and_cannot_adopt_old_identity() {
    let mut fixture = Fixture::new(8);
    let original = fixture.start(1);
    fixture
        .port
        .stop(original.process(), StopMode::Force)
        .expect("stop original");
    let replacement = fixture
        .port
        .start_generation(
            &generation(2, true),
            LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
            profile(),
        )
        .expect("replacement")
        .identity()
        .clone();
    assert_ne!(replacement, original);
    assert_eq!(
        fixture
            .port
            .recover_generation(&generation(2, true), profile())
            .expect("exact replacement"),
        GenerationRecovery::Found(replacement)
    );
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 2);
}

#[test]
fn unknown_user_data_contents_and_changed_images_are_preserved() {
    let mut fixture = Fixture::new(8);
    let sentinel = fixture.root.0.join("user-data/existing-profile");
    std::fs::write(&sentinel, b"do not adopt").expect("sentinel");
    assert!(
        fixture
            .port
            .start_generation(
                &generation(1, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile()
            )
            .is_err()
    );
    assert_eq!(
        std::fs::read(&sentinel).expect("preserved"),
        b"do not adopt"
    );
    std::fs::write(&fixture.profiles[&1].executable, b"unapproved replacement")
        .expect("image drift");
    assert_eq!(
        fixture.port.start_generation(
            &generation(2, false),
            LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
            profile()
        ),
        Err(GenerationStartError::Unsupported)
    );
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 0);
}

#[test]
fn manager_arguments_use_one_exact_unit_and_mandatory_ownership_properties() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    let record = fixture
        .port
        .registry
        .by_handle(identity.process().value())
        .expect("record");
    let args = systemd::launch_arguments(&record, &fixture.profiles[&1]);
    for expected in [
        "--user",
        "--service-type=exec",
        "--remain-after-exit",
        "--expand-environment=no",
        "--property=KillMode=control-group",
        "--property=SendSIGKILL=yes",
        "--property=Restart=no",
        "--property=ProtectControlGroups=yes",
    ] {
        assert!(args.iter().any(|arg| arg == expected));
    }
    assert!(args.contains(&format!("--unit={}", record.unit)));
    let marker = args
        .iter()
        .position(|arg| arg == "--")
        .expect("exec delimiter");
    assert_eq!(
        args[marker + 1],
        fixture.profiles[&1].executable.to_string_lossy()
    );
    assert_eq!(&args[marker + 2..], &["--fixture"]);
    assert!(
        store::GenerationRegistry::open(&fixture.root.0.join("registry.sqlite"), 8).is_err(),
        "exclusive owner"
    );
}

#[test]
fn kernel_birth_parsing_does_not_confuse_parentheses_or_path_traversal() {
    let stat = format!(
        "101 (strange ) name) S {} 12345 0",
        (0..18).map(|_| "1").collect::<Vec<_>>().join(" ")
    );
    assert_eq!(identity::birth(stat.as_bytes()).expect("birth"), 12345);
    assert!(identity::birth(b"invalid").is_err());
    assert!(!identity::valid_group(
        "/user.slice/../foreign.service",
        "foreign.service"
    ));
    assert!(!identity::valid_group(
        "/foreign.service/child",
        "foreign.service"
    ));
}
