// SPDX-License-Identifier: MIT
#![allow(
    clippy::unwrap_used,
    reason = "fixture setup is intentionally fail-fast"
)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::findings;
use crate::config::Policy;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sts2-rust-policy-{}-{unique}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(fs::remove_dir_all(&self.0).is_ok());
    }
}

fn policy() -> Policy {
    Policy::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml")).unwrap()
}

fn root_manifest(members: &str, excludes: &str, unsafe_level: &str, all_level: &str) -> String {
    format!(
        "[workspace]\nmembers = {members}\nexclude = {excludes}\nresolver = \"3\"\n\n[workspace.package]\nedition = \"2024\"\nrust-version = \"1.97.1\"\nlicense = \"MIT\"\n\n[workspace.lints.rust]\nunsafe_code = \"{unsafe_level}\"\n\n[workspace.lints.clippy]\nall = \"{all_level}\"\nunwrap_used = \"deny\"\nexpect_used = \"deny\"\npanic = \"deny\"\ntodo = \"deny\"\nunimplemented = \"deny\"\n"
    )
}

fn member_manifest(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[lints]\nworkspace = true\n"
    )
}

fn has_message(findings: &[crate::diagnostic::Finding], message: &str) -> bool {
    findings.iter().any(|finding| finding.message == message)
}

#[test]
fn weak_workspace_lints_are_rejected() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest("[\"tools/repo-policy\"]", "[]", "allow", "allow"),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );

    let findings = findings(&fixture.0, &policy());
    assert!(has_message(
        &findings,
        "workspace.lints.rust.unsafe_code must be deny or forbid, found allow"
    ));
    assert!(has_message(
        &findings,
        "workspace.lints.clippy.all cannot be allowed"
    ));
}

#[test]
fn source_bin_is_discovered_even_when_bin_is_ignored() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest(
            "[\"tools/repo-policy\", \"src/bin/*\"]",
            "[]",
            "forbid",
            "deny",
        ),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );
    fixture.write("src/bin/fixture/Cargo.toml", &member_manifest("fixture"));

    let findings = findings(&fixture.0, &policy());
    assert!(!findings.iter().any(|finding| {
        finding.path == "src/bin/fixture/Cargo.toml"
            && finding
                .message
                .contains("does not resolve to a Cargo manifest")
    }));
}

#[test]
fn excluded_nested_workspace_still_has_its_own_policy_checked() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest(
            "[\"tools/repo-policy\"]",
            "[\"nested-workspace\"]",
            "forbid",
            "deny",
        ),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );
    fixture.write(
        "nested-workspace/Cargo.toml",
        "[workspace]\nmembers = [\"tools/nested-member\"]\n\n[workspace.package]\nedition = \"2024\"\nrust-version = \"1.97.1\"\nlicense = \"MIT\"\n\n[workspace.lints.rust]\nunsafe_code = \"forbid\"\n\n[workspace.lints.clippy]\nall = \"allow\"\n",
    );
    fixture.write(
        "nested-workspace/tools/nested-member/Cargo.toml",
        "[package]\nname = \"nested-member\"\nversion = \"0.0.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n",
    );

    let findings = findings(&fixture.0, &policy());
    assert!(findings.iter().any(|finding| {
        finding.path == "nested-workspace/Cargo.toml"
            && finding.message == "workspace.lints.clippy.all cannot be allowed"
    }));
    assert!(findings.iter().any(|finding| {
        finding.path == "nested-workspace/tools/nested-member/Cargo.toml"
            && finding
                .message
                .contains("workspace member must inherit the workspace lint policy")
    }));
}

#[test]
fn managed_standards_workspace_requires_adopter_metadata() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest("[\"tools/repo-policy\"]", "[]", "forbid", "deny"),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );
    fixture.write(
        "standards/tools/standards-sync/Cargo.toml",
        "[package]\nname = \"standards-sync\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.97.1\"\nlicense = \"MIT\"\n[workspace]\n",
    );

    let findings = findings(&fixture.0, &policy());
    assert!(findings.iter().any(|finding| {
        finding.rule == "RUST005"
            && finding.path == "standards-profile.toml"
            && finding.message.contains("requires adopter metadata")
    }));
    assert!(findings.iter().any(|finding| {
        finding.rule == "RUST005"
            && finding.path == "standards.lock.json"
            && finding.message.contains("requires adopter metadata")
    }));
}

#[cfg(unix)]
#[test]
fn source_symlink_is_rejected_instead_of_followed() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest("[\"tools/repo-policy\"]", "[]", "forbid", "deny"),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );
    fs::create_dir_all(fixture.0.join("src")).unwrap();
    symlink(
        "../missing-private-source.rs",
        fixture.0.join("src/link.rs"),
    )
    .unwrap();

    let findings = crate::files::symlink_findings(&fixture.0, &policy());
    assert!(findings.iter().any(|finding| {
        finding.rule == "CFG001"
            && finding.path == "src/link.rs"
            && finding.message.contains("symbolic links are not permitted")
    }));
}

#[test]
fn excluded_standalone_manifest_is_rejected() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest(
            "[\"tools/repo-policy\"]",
            "[\"excluded\"]",
            "forbid",
            "deny",
        ),
    );
    fixture.write("Cargo.lock", "# fixture lock\n");
    fixture.write("rust-toolchain.toml", "[toolchain]\nchannel = \"1.97.1\"\n");
    fixture.write(
        "tools/repo-policy/Cargo.toml",
        &member_manifest("repo-policy"),
    );
    fixture.write(
        "excluded/Cargo.toml",
        "[package]\nname = \"excluded\"\nversion = \"0.0.0\"\nedition = \"2024\"\nlicense = \"MIT\"\n",
    );

    let findings = findings(&fixture.0, &policy());
    assert!(findings.iter().any(|finding| {
        finding.rule == "RUST003"
            && finding.path == "excluded/Cargo.toml"
            && finding.message.contains("excluded Cargo manifest")
    }));
}
