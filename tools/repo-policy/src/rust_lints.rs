// SPDX-License-Identifier: MIT

use toml::Value;

use crate::diagnostic::Finding;

pub(super) fn check_workspace(
    workspace: &toml::map::Map<String, Value>,
    relative: &str,
    require_policy_member: bool,
    findings: &mut Vec<Finding>,
) {
    let package = workspace.get("package").and_then(Value::as_table);
    for key in ["edition", "rust-version", "license"] {
        if (require_policy_member || package.is_some())
            && package
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
                .is_none()
        {
            findings.push(Finding::error(
                "RUST001",
                relative,
                format!("workspace.package.{key} must be declared"),
            ));
        }
    }
    let rust_lints = lint_table(workspace, "rust", relative, findings);
    let clippy_lints = lint_table(workspace, "clippy", relative, findings);
    if let Some(lints) = rust_lints {
        require_strong_lint(lints, "rust", "unsafe_code", relative, findings);
    }
    if let Some(lints) = clippy_lints {
        require_non_allow_lint(lints, "clippy", "all", relative, findings);
        for lint in [
            "unwrap_used",
            "expect_used",
            "panic",
            "todo",
            "unimplemented",
        ] {
            require_strong_lint(lints, "clippy", lint, relative, findings);
        }
    }
    let members = workspace.get("members").and_then(Value::as_array);
    if members.is_none_or(Vec::is_empty) {
        findings.push(Finding::error(
            "RUST001",
            relative,
            "workspace.members must contain at least one package",
        ));
    } else if require_policy_member
        && !members.is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str() == Some("tools/repo-policy"))
        })
    {
        findings.push(Finding::error(
            "RUST001",
            relative,
            "workspace must include the target-local repo-policy member",
        ));
    }
}

pub(super) fn check_member(relative: &str, manifest: &Value, findings: &mut Vec<Finding>) {
    let Some(lints) = manifest.get("lints").and_then(Value::as_table) else {
        findings.push(Finding::error(
            "RUST002",
            relative,
            "workspace member must inherit the workspace lint policy",
        ));
        return;
    };
    if lints.get("workspace").and_then(Value::as_bool) != Some(true) {
        findings.push(Finding::error(
            "RUST002",
            relative,
            "workspace member must set lints.workspace = true",
        ));
    }
    if lints.keys().any(|key| key != "workspace") {
        findings.push(Finding::error(
            "RUST003",
            relative,
            "member lint overrides require an explicit reviewed boundary",
        ));
    }
    let Some(package) = manifest.get("package").and_then(Value::as_table) else {
        findings.push(Finding::error(
            "RUST002",
            relative,
            "workspace member lacks [package]",
        ));
        return;
    };
    for key in ["edition", "rust-version", "license"] {
        let inherited = package
            .get(key)
            .and_then(Value::as_table)
            .and_then(|table| table.get("workspace"))
            .and_then(Value::as_bool)
            == Some(true);
        if !inherited {
            findings.push(Finding::error(
                "RUST004",
                relative,
                format!("package.{key} must inherit workspace.package.{key}"),
            ));
        }
    }
}

fn lint_table<'a>(
    workspace: &'a toml::map::Map<String, Value>,
    family: &str,
    relative: &str,
    findings: &mut Vec<Finding>,
) -> Option<&'a toml::map::Map<String, Value>> {
    let Some(lints) = workspace.get("lints").and_then(Value::as_table) else {
        findings.push(Finding::error(
            "RUST001",
            relative,
            "workspace.lints must define inherited lint policy",
        ));
        return None;
    };
    let Some(table) = lints.get(family).and_then(Value::as_table) else {
        findings.push(Finding::error(
            "RUST001",
            relative,
            format!("workspace.lints.{family} must define inherited lint policy"),
        ));
        return None;
    };
    if table.is_empty() {
        findings.push(Finding::error(
            "RUST001",
            relative,
            format!("workspace.lints.{family} must not be empty"),
        ));
    }
    Some(table)
}

fn lint_level(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| {
        value
            .as_table()
            .and_then(|table| table.get("level"))
            .and_then(Value::as_str)
    })
}

fn require_non_allow_lint(
    lints: &toml::map::Map<String, Value>,
    family: &str,
    name: &str,
    relative: &str,
    findings: &mut Vec<Finding>,
) {
    let Some(value) = lints.get(name) else {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} must be configured"),
        ));
        return;
    };
    let Some(level) = lint_level(value) else {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} must contain a lint level"),
        ));
        return;
    };
    if level == "allow" {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} cannot be allowed"),
        ));
    }
}

fn require_strong_lint(
    lints: &toml::map::Map<String, Value>,
    family: &str,
    name: &str,
    relative: &str,
    findings: &mut Vec<Finding>,
) {
    let Some(value) = lints.get(name) else {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} must be deny or forbid"),
        ));
        return;
    };
    let Some(level) = lint_level(value) else {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} must contain a lint level"),
        ));
        return;
    };
    if !matches!(level, "deny" | "forbid") {
        findings.push(Finding::error(
            "RUST004",
            relative,
            format!("workspace.lints.{family}.{name} must be deny or forbid, found {level}"),
        ));
    }
}
