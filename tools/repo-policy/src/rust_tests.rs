// SPDX-License-Identifier: MIT
#![allow(
    clippy::unwrap_used,
    reason = "fixture setup is intentionally fail-fast"
)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
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

    fn cargo(&self, args: &[&str]) -> std::io::Result<Output> {
        Command::new("cargo")
            .arg("+1.97.1")
            .args(args)
            .current_dir(&self.0)
            .env("CARGO_TARGET_DIR", self.0.join("target"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .output()
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

fn policy_with_exact_exemption(fixture: &Fixture, path: &str) -> Policy {
    fixture.write(
        "policy.toml",
        &format!(
            "policy_version = 2\n\n[project]\nrequired_files = []\nignored_directories = []\nignored_path_prefixes = []\n\n[severity]\nmandatory = [\"*\"]\nadvisory = [\"SIZE001\"]\n\n[limits]\nrust_production_preferred = 10\nrust_production_max = 20\nrust_test_preferred = 10\nrust_test_max = 20\ncsharp_production_preferred = 10\ncsharp_production_max = 20\ncsharp_test_preferred = 10\ncsharp_test_max = 20\nworkflow_preferred = 10\nworkflow_max = 20\nmarkdown_preferred = 10\nmarkdown_max = 20\n\n[exemptions]\n\"{path}\" = \"A fixture-specific exact path with compensating evidence.\"\n"
        ),
    );
    Policy::load(&fixture.0.join("policy.toml")).unwrap()
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
fn member_lint_weakening_is_rejected_beside_an_exact_exemption() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &root_manifest(
            "[\"tools/repo-policy\", \"crates/weak-member\", \"crates/valid-member\"]",
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
    fixture.write(
        "crates/weak-member/Cargo.toml",
        "[package]\nname = \"weak-member\"\nversion = \"0.0.0\"\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\n\n[lints]\nworkspace = true\n\n[lints.clippy]\nunwrap_used = \"allow\"\n",
    );
    fixture.write(
        "crates/valid-member/Cargo.toml",
        &member_manifest("valid-member"),
    );

    let policy = policy_with_exact_exemption(&fixture, "crates/weak-member/Cargo.toml");
    assert!(crate::files::exemption_findings(&fixture.0, &policy).is_empty());
    let findings = findings(&fixture.0, &policy);
    assert!(findings.iter().any(|finding| {
        finding.rule == "RUST003"
            && finding.path == "crates/weak-member/Cargo.toml"
            && finding
                .message
                .contains("member lint overrides require an explicit reviewed boundary")
    }));
    assert!(!findings.iter().any(|finding| {
        finding.path == "crates/valid-member/Cargo.toml" && finding.rule.starts_with("RUST00")
    }));
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

#[test]
fn documentation_lane_rejects_a_broken_example_and_accepts_the_fixed_example() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"doc-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.97.1\"\n",
    );
    fixture.write(
        "src/lib.rs",
        "/// Returns the fixture value.\n///\n/// ```\n/// assert_eq!(doc_fixture::value(), 8);\n/// ```\npub fn value() -> u8 { 7 }\n",
    );
    let lock = fixture.cargo(&["generate-lockfile", "--offline"]).unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );

    let broken = fixture
        .cargo(&["test", "--doc", "--offline", "--locked"])
        .unwrap();
    let broken_diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&broken.stdout),
        String::from_utf8_lossy(&broken.stderr)
    );
    assert!(!broken.status.success(), "broken doctest was accepted");
    assert!(
        broken_diagnostic.contains("assertion `left == right` failed"),
        "wrong doctest failure: {broken_diagnostic}"
    );

    fixture.write(
        "src/lib.rs",
        "/// Returns the fixture value.\n///\n/// ```\n/// assert_eq!(doc_fixture::value(), 7);\n/// ```\npub fn value() -> u8 { 7 }\n",
    );
    let fixed = fixture
        .cargo(&["test", "--doc", "--offline", "--locked"])
        .unwrap();
    assert!(
        fixed.status.success(),
        "{}",
        String::from_utf8_lossy(&fixed.stderr)
    );
}

#[test]
fn policy_load_rejects_unreadable_and_malformed_configuration() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("policy-directory")).unwrap();
    let unreadable = Policy::load(&fixture.0.join("policy-directory")).unwrap_err();
    assert!(unreadable.starts_with("cannot read "), "{unreadable}");

    fixture.write("malformed-policy.toml", "policy_version = [");
    let malformed = Policy::load(&fixture.0.join("malformed-policy.toml")).unwrap_err();
    assert!(
        malformed.starts_with("cannot parse policy.toml:")
            || malformed.starts_with("policy_version must be"),
        "{malformed}"
    );
}
