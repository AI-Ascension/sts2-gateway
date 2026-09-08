// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root =
            std::env::temp_dir().join(format!("production-lints-{}-{nonce}", std::process::id()));
        fs::create_dir(&root)?;
        fs::create_dir(root.join("src"))?;
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "production-lint-fixture"
version = "0.0.0"
edition = "2024"
[workspace]
[lints.rust]
unsafe_code = "deny"
[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
"#,
        )?;
        fs::write(
            root.join("src/lib.rs"),
            "pub fn value(input: Option<u8>) -> Option<u8> { input }\n",
        )?;
        let fixture = Self(root);
        let output = fixture.cargo(&["generate-lockfile", "--offline"])?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(fixture)
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

    fn production(&self, source: &str) -> std::io::Result<Output> {
        fs::write(self.0.join("src/lib.rs"), source)?;
        self.cargo(&[
            "clippy",
            "--offline",
            "--workspace",
            "--lib",
            "--bins",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
            "-F",
            "clippy::unwrap_used",
            "-F",
            "clippy::expect_used",
            "-F",
            "clippy::panic",
            "-F",
            "clippy::todo",
            "-F",
            "clippy::unimplemented",
        ])
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn production_gate_rejects_constructs_and_allow_override_but_preserves_test_scope() -> TestResult {
    let fixture = Fixture::new()?;
    let override_source = "#![allow(clippy::unwrap_used)]\npub fn value(input: Option<u8>) -> u8 { input.unwrap() }\n";
    fs::write(fixture.0.join("src/lib.rs"), override_source)?;
    let baseline = fixture.cargo(&[
        "clippy",
        "--offline",
        "--lib",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );

    for (source, lint) in [
        (
            "pub fn value(input: Option<u8>) -> u8 { input.unwrap() }",
            "unwrap_used",
        ),
        (
            "pub fn value(input: Option<u8>) -> u8 { input.expect(\"fixture\") }",
            "expect_used",
        ),
        ("pub fn value() -> u8 { panic!(\"fixture\") }", "panic"),
        ("pub fn value() -> u8 { todo!() }", "todo"),
        ("pub fn value() -> u8 { unimplemented!() }", "unimplemented"),
        (override_source, "unwrap_used"),
    ] {
        let output = fixture.production(source)?;
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {lint}");
        assert!(
            diagnostic.contains(&format!("clippy::{lint}"))
                || diagnostic.contains(&format!("clippy::{}", lint.replace('_', "-"))),
            "wrong failure for {lint}: {diagnostic}"
        );
    }
    let valid = r#"// unwrap, expect, panic! and todo! in a comment are not code.
pub fn value(input: Option<u8>) -> Option<u8> { input }
#[cfg(test)]
mod tests {
    #[test]
    #[allow(clippy::unwrap_used)]
    fn allowed_test_assertion() { assert_eq!(super::value(Some(7)).unwrap(), 7); }
}
"#;
    let output = fixture.production(valid)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tests = fixture.cargo(&[
        "clippy",
        "--offline",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    assert!(
        tests.status.success(),
        "{}",
        String::from_utf8_lossy(&tests.stderr)
    );
    Ok(())
}

#[test]
fn compiler_rejects_unsafe_code_outside_an_approved_boundary() -> TestResult {
    let fixture = Fixture::new()?;
    let output = fixture.production(
        "pub fn value() -> u8 {\n    let mut output = 0;\n    unsafe { std::ptr::write(&mut output, 7); }\n    output\n}\n",
    )?;
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "unsafe fixture was accepted");
    assert!(
        diagnostic.contains("unsafe_code")
            || diagnostic.contains("unsafe-code")
            || diagnostic.contains("usage of an `unsafe` block"),
        "wrong unsafe-code failure: {diagnostic}"
    );
    Ok(())
}
