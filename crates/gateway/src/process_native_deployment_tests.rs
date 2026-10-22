// SPDX-License-Identifier: MIT

use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn configuration(fixture: &Fixture) -> serde_json::Value {
    let profile = &fixture.profiles[&1];
    json!({
        "schema_version": "sts2-gateway-systemd-process/v1",
        "registry_path": fixture.root.0.join("deployment.sqlite"),
        "runtime_directory": fixture.root.0,
        "systemctl_path": profile.executable,
        "systemd_run_path": profile.executable,
        "profiles": [{
            "profile_id": 1,
            "install_root": fixture.root.0,
            "executable": profile.executable,
            "executable_sha256": profile.image_digest,
            "user_data_root": profile.user_data,
            "arguments": ["--user-data={user_data}", "$LITERAL;no-shell"]
        }]
    })
}

#[test]
fn explicit_configuration_binds_only_complete_exact_approved_profiles() {
    let fixture = Fixture::new(8);
    let path = fixture.root.0.join("deployment.json");
    let value = configuration(&fixture);
    std::fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("configuration");
    let resolved = configuration::NativeConfiguration::read(&path, &[profile()])
        .expect("approved configuration");
    assert_eq!(
        resolved.profiles[&1].arguments,
        [
            format!("--user-data={}", fixture.profiles[&1].user_data.display()),
            "$LITERAL;no-shell".to_owned()
        ]
    );
    for corruption in 0..7 {
        let mut invalid = value.clone();
        match corruption {
            0 => invalid["profiles"][0]["profile_id"] = json!(2),
            1 => invalid["profiles"][0]["executable_sha256"] = json!("0".repeat(64)),
            2 => invalid["profiles"][0]["arguments"] = json!(["%n"]),
            3 => invalid["profiles"][0]["unrecognized"] = json!(true),
            4 => invalid["profiles"] = json!([]),
            5 => invalid["profiles"][0]["executable"] = json!(fixture.root.0.join("../foreign")),
            _ => invalid["schema_version"] = json!("unsupported"),
        }
        std::fs::write(&path, serde_json::to_vec(&invalid).expect("json")).expect("configuration");
        assert!(configuration::NativeConfiguration::read(&path, &[profile()]).is_err());
    }
    assert_eq!(fixture.controller.0.lock().expect("state").starts, 0);
}

#[cfg(unix)]
#[test]
fn approved_profiles_must_have_disjoint_writable_roots() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new(8);
    let sibling = fixture.root.0.join("sibling-data");
    let nested = fixture.profiles[&1].user_data.join("nested");
    for path in [&sibling, &nested] {
        std::fs::create_dir(path).expect("profile directory");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .expect("private profile");
    }
    let second = LaunchProfile::try_new(
        crate::LaunchProfileId::new(2),
        profile().executable(),
        crate::UserDataConfig::new(45),
        profile().policy(),
    )
    .expect("second approved profile");
    let approved = [profile(), second];
    let path = fixture.root.0.join("deployment.json");
    let mut value = configuration(&fixture);
    let mut other = value["profiles"][0].clone();
    other["profile_id"] = json!(2);
    other["user_data_root"] = json!(sibling);
    value["profiles"]
        .as_array_mut()
        .expect("profiles")
        .push(other);
    std::fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("configuration");
    configuration::NativeConfiguration::read(&path, &approved).expect("disjoint roots");
    for reverse in [false, true] {
        let mut overlapping = value.clone();
        overlapping["profiles"][usize::from(reverse)]["user_data_root"] =
            json!(fixture.profiles[&1].user_data);
        overlapping["profiles"][usize::from(!reverse)]["user_data_root"] = json!(nested);
        std::fs::write(&path, serde_json::to_vec(&overlapping).expect("json"))
            .expect("configuration");
        assert!(configuration::NativeConfiguration::read(&path, &approved).is_err());
    }
}

#[test]
fn writable_roots_cannot_contain_registry_configuration_images_or_owner_journals() {
    let fixture = Fixture::new(8);
    let path = fixture.root.0.join("deployment.json");
    let value = configuration(&fixture);
    let image = fixture.profiles[&1].user_data.join("image");
    std::fs::copy(&fixture.profiles[&1].executable, &image).expect("fixture image");
    for protected in [
        "registry_path",
        "systemctl_path",
        "systemd_run_path",
        "image",
        "configuration",
    ] {
        let mut invalid = value.clone();
        let config_path = if protected == "configuration" {
            fixture.profiles[&1].user_data.join("native.json")
        } else {
            path.clone()
        };
        match protected {
            "image" => invalid["profiles"][0]["executable"] = json!(image),
            "configuration" => (),
            "registry_path" => {
                invalid[protected] = json!(fixture.profiles[&1].user_data.join("native.sqlite"))
            }
            _ => invalid[protected] = json!(image),
        }
        std::fs::write(&config_path, serde_json::to_vec(&invalid).expect("json"))
            .expect("configuration");
        assert!(configuration::NativeConfiguration::read(&config_path, &[profile()]).is_err());
    }
    std::fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("configuration");
    let resolved =
        configuration::NativeConfiguration::read(&path, &[profile()]).expect("safe configuration");
    assert!(
        resolved
            .protect_paths(&[&fixture.profiles[&1].user_data.join("gateway-journal")])
            .is_err()
    );
    resolved
        .protect_paths(&[&fixture.root.0.join("gateway-journal")])
        .expect("separate owner state");
}

#[cfg(unix)]
#[test]
fn symlink_user_data_and_non_executable_images_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new(8);
    let path = fixture.root.0.join("deployment.json");
    let mut value = configuration(&fixture);
    symlink(
        &fixture.profiles[&1].user_data,
        fixture.root.0.join("alias"),
    )
    .expect("fixture symlink");
    value["profiles"][0]["user_data_root"] = json!(fixture.root.0.join("alias"));
    std::fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("configuration");
    assert!(configuration::NativeConfiguration::read(&path, &[profile()]).is_err());
    std::fs::set_permissions(
        &fixture.profiles[&1].executable,
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("remove exec permission");
    assert!(fixture.profiles[&1].verify_image().is_err());
}

#[test]
fn terminal_unit_can_clear_live_fields_only_with_retained_empty_cgroup_evidence() {
    let mut fixture = Fixture::new(8);
    let identity = fixture.start(1);
    let record = fixture
        .port
        .registry
        .by_handle(identity.process().value())
        .expect("record");
    let mut properties: BTreeMap<String, String> = [
        ("LoadState", "loaded"),
        ("Transient", "yes"),
        ("KillMode", "control-group"),
        ("SendSIGKILL", "yes"),
        ("Restart", "no"),
        ("Delegate", "no"),
        ("ProtectControlGroups", "yes"),
        ("RemainAfterExit", "yes"),
        ("ExitType", "cgroup"),
        ("ActiveState", "inactive"),
        ("SubState", "dead"),
        ("Job", "0"),
        ("MainPID", "0"),
        ("ControlGroup", ""),
        ("InvocationID", ""),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    properties.insert(
        "Description".to_owned(),
        format!("sts2-generation:{}:{}", record.key, record.binding_digest),
    );
    assert!(systemd::properties::verify(&record, &mut properties.clone(), |_| Ok(false)).is_err());
    systemd::properties::verify(&record, &mut properties.clone(), |_| Ok(true))
        .expect("retained proof");
    for field in [
        "Job",
        "MainPID",
        "InvocationID",
        "ControlGroup",
        "Description",
    ] {
        let mut invalid = properties.clone();
        invalid.insert(field.to_owned(), "foreign".to_owned());
        assert!(systemd::properties::verify(&record, &mut invalid, |_| Ok(true)).is_err());
    }
    properties.insert("ActiveState".to_owned(), "activating".to_owned());
    assert!(systemd::properties::verify(&record, &mut properties, |_| Ok(true)).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn helper_children_are_deadline_bounded_reaped_and_output_is_private() {
    let fixture = Fixture::new(8);
    let started = Instant::now();
    assert!(
        command::run(
            std::path::Path::new("/bin/sleep"),
            &["5".to_owned()],
            &fixture.root.0,
            &fixture.root.0,
            Duration::from_millis(20)
        )
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let literal = "$UNEXPANDED;no-shell";
    let output = command::run(
        std::path::Path::new("/usr/bin/printf"),
        &["%s".to_owned(), literal.to_owned()],
        &fixture.root.0,
        &fixture.root.0,
        Duration::from_secs(1),
    )
    .expect("bounded child");
    assert!(output.success);
    assert_eq!(output.bytes, literal.as_bytes());
    assert!(
        !std::fs::read_dir(&fixture.root.0)
            .expect("private scratch")
            .any(|entry| entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with("process-command-"))
    );
    for version in [b"systemd 253\n".as_slice(), b"unknown", b"0"] {
        assert!(
            command::version(&command::CommandOutput {
                success: true,
                bytes: version.to_vec()
            })
            .is_err()
        );
    }
    assert_eq!(
        command::version(&command::CommandOutput {
            success: true,
            bytes: b"systemd 254 (test)\n".to_vec()
        })
        .expect("qualified version"),
        254
    );
}
