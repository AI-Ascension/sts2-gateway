// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::diagnostic::Finding;
use crate::files::relative_text;

const SPDX: &str = "SPDX-License-Identifier: MIT";

pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_root_license(root, &mut findings);
    for path in files {
        if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("rs" | "cs")
        ) {
            check_source_header(root, path, &mut findings);
        }
        if path.file_name().and_then(|value| value.to_str()) == Some("Cargo.toml") {
            check_manifest(root, path, &mut findings);
        }
    }
    findings
}

fn check_root_license(root: &Path, findings: &mut Vec<Finding>) {
    let path = root.join("LICENSE");
    match fs::read_to_string(&path) {
        Ok(text) if text.starts_with("MIT License\n") => {}
        Ok(_) => findings.push(Finding::error(
            "LIC001",
            "LICENSE",
            "root license must use the MIT text",
        )),
        Err(error) => findings.push(Finding::error(
            "LIC001",
            "LICENSE",
            format!("cannot read root license: {error}"),
        )),
    }
}

fn check_source_header(root: &Path, path: &Path, findings: &mut Vec<Finding>) {
    let relative = relative_text(root, path);
    match fs::read_to_string(path) {
        Ok(text) if text.lines().take(8).any(|line| line.contains(SPDX)) => {}
        Ok(_) => findings.push(Finding::error(
            "LIC002",
            &relative,
            format!("missing {SPDX} in first eight lines"),
        )),
        Err(error) => findings.push(Finding::error(
            "LIC002",
            &relative,
            format!("cannot read source: {error}"),
        )),
    }
}

fn check_manifest(root: &Path, path: &Path, findings: &mut Vec<Finding>) {
    let relative = relative_text(root, path);
    let result = fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| toml::from_str::<Value>(&text).map_err(|error| error.to_string()));
    match result {
        Ok(value) if valid_manifest_license(root, path, &value) => {}
        Ok(_) => findings.push(Finding::error(
            "LIC003",
            &relative,
            "Cargo package license must be MIT or inherited from the MIT workspace",
        )),
        Err(error) => findings.push(Finding::error(
            "LIC003",
            &relative,
            format!("cannot parse Cargo manifest: {error}"),
        )),
    }
}

fn valid_manifest_license(root: &Path, path: &Path, value: &Value) -> bool {
    let Some(table) = value.as_table() else {
        return false;
    };
    if path == root.join("Cargo.toml") {
        return table
            .get("workspace")
            .and_then(Value::as_table)
            .and_then(|workspace| workspace.get("package"))
            .and_then(Value::as_table)
            .and_then(|package| package.get("license"))
            .and_then(Value::as_str)
            == Some("MIT");
    }
    let Some(license) = table
        .get("package")
        .and_then(Value::as_table)
        .and_then(|package| package.get("license"))
    else {
        return false;
    };
    license.as_str() == Some("MIT")
        || license
            .as_table()
            .and_then(|table| table.get("workspace"))
            .and_then(Value::as_bool)
            == Some(true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use toml::Value;

    use super::valid_manifest_license;

    #[test]
    fn accepts_workspace_and_inherited_mit() -> Result<(), String> {
        let root = Path::new("repository");
        let workspace =
            toml::from_str::<Value>("[workspace]\n[workspace.package]\nlicense = \"MIT\"\n")
                .map_err(|error| error.to_string())?;
        let package = toml::from_str::<Value>("[package]\nlicense.workspace = true\n")
            .map_err(|error| error.to_string())?;
        assert!(valid_manifest_license(
            root,
            &root.join("Cargo.toml"),
            &workspace
        ));
        assert!(valid_manifest_license(
            root,
            &root.join("crate/Cargo.toml"),
            &package
        ));
        Ok(())
    }
}
