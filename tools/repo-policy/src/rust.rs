// SPDX-License-Identifier: MIT

use std::fs;
use std::path::Path;

use toml::Value;

use crate::diagnostic::Finding;

pub(crate) fn findings(root: &Path) -> Vec<Finding> {
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for relative in ["Cargo.lock", "rust-toolchain.toml"] {
        if !root.join(relative).is_file() {
            findings.push(Finding::error(
                "RUST001",
                relative,
                "required when a Cargo workspace exists",
            ));
        }
    }
    let manifest = parse(&manifest_path, "Cargo.toml", &mut findings);
    let toolchain = parse(
        &root.join("rust-toolchain.toml"),
        "rust-toolchain.toml",
        &mut findings,
    );
    if let Some(manifest) = manifest {
        check_workspace(&manifest, &mut findings);
        if let Some(toolchain) = toolchain {
            check_toolchain_match(&manifest, &toolchain, &mut findings);
        }
    }
    findings
}

fn parse(path: &Path, relative: &str, findings: &mut Vec<Finding>) -> Option<Value> {
    match fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| toml::from_str::<Value>(&text).map_err(|error| error.to_string()))
    {
        Ok(value) => Some(value),
        Err(error) => {
            findings.push(Finding::error(
                "RUST001",
                relative,
                format!("cannot parse Rust configuration: {error}"),
            ));
            None
        }
    }
}

fn check_workspace(manifest: &Value, findings: &mut Vec<Finding>) {
    let workspace = manifest.get("workspace");
    let package = workspace.and_then(|value| value.get("package"));
    for key in ["edition", "rust-version", "license"] {
        if package
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .is_none()
        {
            findings.push(Finding::error(
                "RUST001",
                "Cargo.toml",
                format!("workspace.package.{key} must be declared"),
            ));
        }
    }
    if workspace
        .and_then(|value| value.get("lints"))
        .and_then(Value::as_table)
        .is_none_or(toml::map::Map::is_empty)
    {
        findings.push(Finding::error(
            "RUST001",
            "Cargo.toml",
            "workspace.lints must define inherited lint policy",
        ));
    }
}

fn check_toolchain_match(manifest: &Value, toolchain: &Value, findings: &mut Vec<Finding>) {
    let rust_version = manifest
        .get("workspace")
        .and_then(|value| value.get("package"))
        .and_then(|value| value.get("rust-version"))
        .and_then(Value::as_str);
    let channel = toolchain
        .get("toolchain")
        .and_then(|value| value.get("channel"))
        .and_then(Value::as_str);
    if rust_version != channel {
        findings.push(Finding::error(
            "RUST001",
            "rust-toolchain.toml",
            format!("toolchain {channel:?} does not match workspace rust-version {rust_version:?}"),
        ));
    }
}
