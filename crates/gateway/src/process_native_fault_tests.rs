// SPDX-License-Identifier: MIT

use super::*;
use rusqlite::Connection;

fn start_result(
    fixture: &mut Fixture,
    operation: u64,
) -> Result<ProcessLaunch, GenerationStartError> {
    fixture.port.start_generation(
        &generation(operation, false),
        LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
        profile(),
    )
}

#[test]
fn durable_dispatch_failure_and_lost_identity_commit_never_repeat_a_launch() {
    for fail_after_start in [false, true] {
        let mut fixture = Fixture::new(8);
        let database =
            Connection::open(fixture.root.0.join("registry.sqlite")).expect("fault port");
        let condition = if fail_after_start {
            "json_extract(CAST(NEW.record AS TEXT),'$.identity') IS NOT NULL"
        } else {
            "json_extract(CAST(NEW.record AS TEXT),'$.dispatched')=1"
        };
        database
            .execute_batch(&format!(
                "CREATE TRIGGER inject BEFORE UPDATE ON process_generations WHEN {condition}
             BEGIN SELECT RAISE(ABORT,'injected durable write failure'); END;"
            ))
            .expect("write fault");
        assert!(start_result(&mut fixture, 1).is_err());
        assert_eq!(
            fixture.controller.0.lock().expect("state").starts,
            usize::from(fail_after_start)
        );
        database
            .execute_batch("DROP TRIGGER inject;")
            .expect("repair storage");
        let recovery = fixture
            .port
            .recover_generation(&generation(1, false), profile())
            .expect("recovery");
        assert_eq!(
            matches!(recovery, GenerationRecovery::Found(_)),
            fail_after_start
        );
        let _ = start_result(&mut fixture, 1);
        assert_eq!(
            fixture.controller.0.lock().expect("state").starts,
            usize::from(fail_after_start)
        );
    }
}

#[test]
fn stop_requires_a_durable_intent_and_a_durable_absence_result() {
    for fail_after_stop in [false, true] {
        let mut fixture = Fixture::new(8);
        let identity = fixture.start(1);
        let database =
            Connection::open(fixture.root.0.join("registry.sqlite")).expect("fault port");
        let field = if fail_after_stop {
            "absent"
        } else {
            "stopping"
        };
        database
            .execute_batch(&format!(
                "CREATE TRIGGER inject BEFORE UPDATE ON process_generations
             WHEN json_extract(CAST(NEW.record AS TEXT),'$.{field}')=1
             BEGIN SELECT RAISE(ABORT,'injected durable write failure'); END;"
            ))
            .expect("write fault");
        assert_eq!(
            fixture.port.stop(identity.process(), StopMode::Force),
            Err(ProcessFault::Unavailable)
        );
        assert_eq!(
            fixture.controller.0.lock().expect("state").stops,
            usize::from(fail_after_stop)
        );
        database
            .execute_batch("DROP TRIGGER inject;")
            .expect("repair storage");
        fixture
            .port
            .stop(identity.process(), StopMode::Force)
            .expect("reconcile stop");
        assert_eq!(fixture.controller.0.lock().expect("state").stops, 1);
        assert_eq!(
            fixture
                .port
                .inspect(identity.process())
                .expect("durable absence"),
            ProcessState::Exited { code: None }
        );
    }
}

#[test]
fn retained_generation_excludes_concurrent_use_of_its_user_data_namespace() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    assert!(start_result(&mut fixture, 2).is_err());
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 1);
    assert_eq!(
        fixture.port.inspect(identity.process()).expect("original"),
        ProcessState::Running
    );
    assert_eq!(
        fixture.port.start_generation(
            &generation(1, false),
            LaunchSpec::for_profile(InstanceId::new(8), profile().id()),
            profile()
        ),
        Err(GenerationStartError::Unsupported)
    );
}

#[test]
fn first_binding_cannot_adopt_a_recycled_foreign_pid_from_a_stale_group_snapshot() {
    let mut fixture = Fixture::new(8);
    fixture.controller.0.lock().expect("state").lost_ack = true;
    assert!(start_result(&mut fixture, 1).is_err());
    {
        let mut state = fixture.controller.0.lock().expect("state");
        let UnitObservation::Present { members, .. } =
            state.units.values_mut().next().expect("unit")
        else {
            unreachable!()
        };
        // PID, birth and approved image remain stable throughout the subsequent observation.
        members[0].control_group = "/foreign.slice/control.service".to_owned();
    }
    assert_eq!(
        fixture
            .port
            .recover_generation(&generation(1, false), profile()),
        Err(ProcessFault::IdentityMismatch)
    );
    assert!(start_result(&mut fixture, 1).is_err());
    let record = fixture
        .port
        .registry
        .by_key(&store::generation_key(&generation(1, false)))
        .expect("record")
        .expect("reserved");
    assert!(record.identity.is_none());
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 1);
    assert_eq!(fixture.controller.0.lock().expect("state").stops, 0);
}

#[test]
fn a_recreated_registry_cannot_reuse_an_existing_owned_user_data_root() {
    let mut fixture = Fixture::new(8);
    fixture.start(1);
    let old_registry = fixture.port.registry.identity().to_owned();
    let Fixture {
        port,
        controller,
        profiles,
        root,
    } = fixture;
    drop(port);
    std::fs::remove_file(root.0.join("registry.sqlite")).expect("simulate lost registry");
    let registry =
        store::GenerationRegistry::open(&root.0.join("registry.sqlite"), 8).expect("new registry");
    assert_ne!(registry.identity(), old_registry);
    let mut reopened = port::NativeProcessPort::new(profiles, controller.clone(), registry);
    assert!(
        reopened
            .start_generation(
                &generation(2, false),
                LaunchSpec::for_profile(InstanceId::new(7), profile().id()),
                profile()
            )
            .is_err()
    );
    assert_eq!(controller.0.lock().expect("state").starts, 1);
    drop(reopened);
}

#[test]
fn row_identity_corruption_is_rejected_during_access_and_reopen() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    let database = Connection::open(fixture.root.0.join("registry.sqlite")).expect("fault port");
    database
        .execute(
            "UPDATE process_generations SET generation_key=?1",
            ["f".repeat(64)],
        )
        .expect("corrupt key");
    assert_eq!(
        fixture.port.stop(identity.process(), StopMode::Force),
        Err(ProcessFault::IdentityMismatch)
    );
    assert_eq!(fixture.controller.0.lock().expect("state").stops, 0);
    let Fixture { port, root, .. } = fixture;
    drop(database);
    drop(port);
    assert!(store::GenerationRegistry::open(&root.0.join("registry.sqlite"), 8).is_err());
}

#[test]
fn an_unrelated_database_is_preserved_and_never_adopted() {
    let fixture = Fixture::new(8);
    let path = fixture.root.0.join("unrelated.sqlite");
    let database = Connection::open(&path).expect("fixture database");
    database
        .execute_batch(
            "CREATE TABLE valued_data(value TEXT); INSERT INTO valued_data VALUES('preserve');",
        )
        .expect("sentinel");
    assert!(store::GenerationRegistry::open(&path, 8).is_err());
    assert_eq!(
        database
            .query_row("SELECT value FROM valued_data", [], |row| row
                .get::<_, String>(0))
            .expect("unchanged"),
        "preserve"
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='process_generations'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .expect("no adopted table"),
        0
    );
}
