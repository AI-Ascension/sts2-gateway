// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::config::Policy;
use crate::diagnostic::Finding;

const MANAGED_STANDARDS_PATH: &str = "standards/tools/standards-sync";
const MANAGED_STANDARDS_MANIFEST: &str = "standards/tools/standards-sync/Cargo.toml";
const MANAGED_STANDARDS_PROFILE: &str = "standards-profile.toml";
const MANAGED_STANDARDS_LOCK: &str = "standards.lock.json";

#[path = "rust_lints.rs"]
mod lints;
#[path = "rust_paths.rs"]
mod paths;
use lints::{check_member, check_workspace};
use paths::{
    ignored_prefix, is_excluded, is_rust_binary_sources_dir, is_safe_relative_path, matches_path,
    path_array, relative,
};

#[cfg(test)]
#[path = "rust_tests.rs"]
mod tests;

pub(crate) fn findings(root: &Path, policy: &Policy) -> Vec<Finding> {
    let manifest_path = root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    check_managed_standards(root, policy, &mut findings);
    for relative in ["Cargo.lock", "rust-toolchain.toml"] {
        if !root.join(relative).is_file() {
            findings.push(Finding::error(
                "RUST001",
                relative,
                "required when a Cargo workspace exists",
            ));
        }
    }
    let Some(root_manifest) = read_manifest(&manifest_path, "Cargo.toml", &mut findings) else {
        return findings;
    };
    let Some(workspace) = root_manifest.get("workspace").and_then(Value::as_table) else {
        findings.push(Finding::error(
            "RUST001",
            "Cargo.toml",
            "root Cargo.toml must define a workspace",
        ));
        return findings;
    };
    check_workspace(workspace, "Cargo.toml", true, &mut findings);
    let Some(toolchain) = read_manifest(
        &root.join("rust-toolchain.toml"),
        "rust-toolchain.toml",
        &mut findings,
    ) else {
        return findings;
    };
    check_toolchain_match(&root_manifest, &toolchain, &mut findings);

    let members = path_array(workspace.get("members"), "workspace.members", &mut findings);
    let excludes = path_array(workspace.get("exclude"), "workspace.exclude", &mut findings);
    validate_excludes(&excludes, &mut findings);
    let manifests = collect_manifests(root, policy, &mut findings);
    check_root_members(&members, &excludes, &manifests, &mut findings);
    check_manifest_policies(&members, &excludes, manifests, &mut findings);
    findings
}

fn check_managed_standards(root: &Path, policy: &Policy, findings: &mut Vec<Finding>) {
    let manifest = root.join(MANAGED_STANDARDS_MANIFEST);
    let Ok(metadata) = fs::symlink_metadata(&manifest) else {
        return;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        findings.push(Finding::error(
            "RUST005",
            MANAGED_STANDARDS_MANIFEST,
            "managed standards Cargo manifest must be a regular file",
        ));
        return;
    }
    if !policy
        .ignored_path_prefixes
        .contains(MANAGED_STANDARDS_PATH)
    {
        findings.push(Finding::error(
            "RUST005",
            MANAGED_STANDARDS_PATH,
            "managed standards workspace must use the exact configured ignore path",
        ));
    }
    for path in [MANAGED_STANDARDS_PROFILE, MANAGED_STANDARDS_LOCK] {
        let candidate = root.join(path);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => findings.push(Finding::error(
                "RUST005",
                path,
                "managed standards workspace requires a regular adopter metadata file",
            )),
            Err(_) => findings.push(Finding::error(
                "RUST005",
                path,
                "managed standards workspace requires adopter metadata",
            )),
        }
    }
}

fn validate_excludes(excludes: &[String], findings: &mut Vec<Finding>) {
    for exclude in excludes {
        if !is_safe_relative_path(exclude) {
            findings.push(Finding::error(
                "RUST002",
                exclude,
                "workspace exclusion must be a safe repository-relative path",
            ));
        }
    }
}

fn check_root_members(
    members: &[String],
    excludes: &[String],
    manifests: &[(String, PathBuf, Value)],
    findings: &mut Vec<Finding>,
) {
    let mut declared = BTreeSet::new();
    for (relative, _, value) in manifests {
        if relative == "Cargo.toml" {
            continue;
        }
        let directory = relative
            .strip_suffix("/Cargo.toml")
            .unwrap_or(relative.as_str());
        if is_excluded(directory, excludes) {
            if let Some(workspace) = value.get("workspace").and_then(Value::as_table) {
                // An explicit exclusion keeps a nested workspace out of the root package
                // graph, but it must still carry its own lint and member policy.
                check_workspace(workspace, relative, false, findings);
            } else {
                findings.push(Finding::error(
                    "RUST003",
                    relative,
                    "excluded Cargo manifest must declare a nested workspace",
                ));
            }
            continue;
        }
        if members
            .iter()
            .any(|pattern| matches_path(directory, pattern))
        {
            declared.insert(directory.to_owned());
        } else {
            let rule = if value.get("workspace").is_some() {
                "RUST003"
            } else {
                "RUST002"
            };
            let message = if rule == "RUST003" {
                "nested workspace must be listed in workspace.exclude"
            } else {
                "Cargo manifest is not a declared workspace member"
            };
            findings.push(Finding::error(rule, relative, message));
        }
    }
    for member in members {
        if !is_safe_relative_path(member) {
            findings.push(Finding::error(
                "RUST002",
                member,
                "workspace member must be a safe repository-relative path",
            ));
        } else if !declared
            .iter()
            .any(|path| path == member || matches_path(path, member))
        {
            findings.push(Finding::error(
                "RUST002",
                member,
                "workspace member pattern does not resolve to a Cargo manifest",
            ));
        }
    }
}

fn check_manifest_policies(
    members: &[String],
    excludes: &[String],
    manifests: Vec<(String, PathBuf, Value)>,
    findings: &mut Vec<Finding>,
) {
    let nested_workspaces = nested_workspaces(excludes, &manifests);
    for (relative, _, value) in manifests {
        if relative == "Cargo.toml" {
            continue;
        }
        let directory = relative
            .strip_suffix("/Cargo.toml")
            .unwrap_or(relative.as_str());
        if is_excluded(directory, excludes) {
            check_nested_member(&relative, directory, &value, &nested_workspaces, findings);
        } else if members
            .iter()
            .any(|pattern| matches_path(directory, pattern))
        {
            check_member(&relative, &value, findings);
        }
    }
}

fn nested_workspaces(
    excludes: &[String],
    manifests: &[(String, PathBuf, Value)],
) -> Vec<(String, toml::map::Map<String, Value>)> {
    manifests
        .iter()
        .filter_map(|(relative, _, value)| {
            let directory = relative.strip_suffix("/Cargo.toml")?;
            is_excluded(directory, excludes)
                .then(|| value.get("workspace").and_then(Value::as_table))
                .flatten()
                .map(|workspace| (directory.to_owned(), workspace.clone()))
        })
        .collect()
}

fn check_nested_member(
    relative: &str,
    directory: &str,
    value: &Value,
    nested_workspaces: &[(String, toml::map::Map<String, Value>)],
    findings: &mut Vec<Finding>,
) {
    if value.get("workspace").is_some() {
        return;
    }
    let Some((nested_directory, workspace)) = nested_workspaces
        .iter()
        .filter(|(candidate, _)| {
            directory
                .strip_prefix(candidate)
                .and_then(|suffix| suffix.strip_prefix('/'))
                .is_some()
        })
        .max_by_key(|(candidate, _)| candidate.len())
    else {
        return;
    };
    let nested_members = workspace
        .get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let member_path = directory
        .strip_prefix(nested_directory)
        .unwrap_or(directory)
        .trim_matches('/');
    if nested_members
        .clone()
        .any(|pattern| matches_path(member_path, pattern))
    {
        check_member(relative, value, findings);
    } else {
        findings.push(Finding::error(
            "RUST002",
            relative,
            "nested workspace member is not declared by its workspace",
        ));
    }
}

fn collect_manifests(
    root: &Path,
    policy: &Policy,
    findings: &mut Vec<Finding>,
) -> Vec<(String, PathBuf, Value)> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            findings.push(Finding::error(
                "RUST001",
                &relative(root, &directory),
                "cannot read directory while discovering Cargo manifests",
            ));
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            let relative_path = relative(root, &path);
            if metadata.is_dir() {
                let ignored = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        (policy.ignored_directories.contains(name)
                            && !is_rust_binary_sources_dir(&path))
                            || ignored_prefix(&relative_path, policy)
                    });
                if !ignored {
                    pending.push(path);
                }
            } else if metadata.is_file()
                && path.file_name().is_some_and(|name| name == "Cargo.toml")
                && !ignored_prefix(&relative_path, policy)
                && let Some(value) = read_manifest(&path, &relative_path, findings)
            {
                paths.push((relative_path, path, value));
            }
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths
}

fn read_manifest(path: &Path, relative: &str, findings: &mut Vec<Finding>) -> Option<Value> {
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
