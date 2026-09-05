// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::{Policy, SizeCategory};
use crate::diagnostic::Finding;

const PYTHON_MANIFESTS: &[&str] = &[
    ".python-version",
    "Pipfile",
    "Pipfile.lock",
    "poetry.lock",
    "pyproject.toml",
    "uv.lock",
];

pub(crate) fn collect(root: &Path, policy: &Policy) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot read directory entry: {error}"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("cannot relativize {}: {error}", path.display()))?;
            if metadata.is_dir() {
                if !ignored_directory(relative, policy) {
                    pending.push(path);
                }
            } else if metadata.is_file() && !ignored_prefix(relative, policy) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn required_file_findings(root: &Path, policy: &Policy) -> Vec<Finding> {
    policy
        .required_files
        .iter()
        .filter(|relative| !root.join(*relative).is_file())
        .map(|relative| Finding::error("DOC001", relative, "required file is missing"))
        .collect()
}

pub(crate) fn exemption_findings(root: &Path, policy: &Policy) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (relative, reason) in &policy.exemptions {
        let path = Path::new(relative);
        if path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exemption must be an exact repository-relative path",
            ));
        } else if reason.trim().len() < 20 {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exemption reason must contain at least 20 characters",
            ));
        } else if !root.join(path).is_file() {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exempted file does not exist",
            ));
        }
    }
    findings
}

pub(crate) fn language_findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    files
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase);
            let is_python = matches!(extension.as_deref(), Some("py" | "pyi" | "pyc" | "pyo"))
                || PYTHON_MANIFESTS.contains(&name)
                || (name.starts_with("requirements")
                    && Path::new(name)
                        .extension()
                        .is_some_and(|value| value.eq_ignore_ascii_case("txt")));
            is_python.then(|| {
                Finding::error(
                    "LANG001",
                    &relative_text(root, path),
                    "Python source or package metadata is prohibited; repository tooling is Rust",
                )
            })
        })
        .collect()
}

pub(crate) fn size_findings(
    root: &Path,
    files: &[PathBuf],
    policy: &Policy,
) -> (usize, Vec<Finding>) {
    let exempt: BTreeSet<&str> = policy.exemptions.keys().map(String::as_str).collect();
    let mut checked = 0;
    let mut findings = Vec::new();
    for path in files {
        let relative = relative_text(root, path);
        let Some(category) = size_category(Path::new(&relative)) else {
            continue;
        };
        if exempt.contains(relative.as_str()) {
            continue;
        }
        checked += 1;
        match fs::read_to_string(path) {
            Ok(text) => check_size(&relative, &text, policy, category, &mut findings),
            Err(error) => findings.push(Finding::error(
                "SIZE001",
                &relative,
                format!("cannot read UTF-8 text: {error}"),
            )),
        }
    }
    (checked, findings)
}

fn check_size(
    relative: &str,
    text: &str,
    policy: &Policy,
    category: SizeCategory,
    findings: &mut Vec<Finding>,
) {
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    let budget = policy.budget(category);
    if lines > budget.maximum {
        findings.push(Finding::error(
            "SIZE001",
            relative,
            format!(
                "{lines} nonblank lines exceeds hard maximum {}",
                budget.maximum
            ),
        ));
    } else if lines > budget.preferred {
        findings.push(Finding::warning(
            "SIZE001",
            relative,
            format!(
                "{lines} nonblank lines exceeds preferred maximum {}",
                budget.preferred
            ),
        ));
    }
}

fn size_category(relative: &Path) -> Option<SizeCategory> {
    let text = relative.to_string_lossy().replace('\\', "/");
    let name = relative.file_name()?.to_string_lossy().to_ascii_lowercase();
    if text.starts_with(".github/workflows/")
        && matches!(
            relative.extension().and_then(|item| item.to_str()),
            Some("yml" | "yaml")
        )
    {
        return Some(SizeCategory::Workflow);
    }
    match relative.extension().and_then(|item| item.to_str()) {
        Some("rs") if is_test_path(relative, &name, &["_test.rs", "_tests.rs"]) => {
            Some(SizeCategory::RustTest)
        }
        Some("rs") => Some(SizeCategory::RustProduction),
        Some("cs") if is_test_path(relative, &name, &["test.cs", "tests.cs"]) => {
            Some(SizeCategory::CsharpTest)
        }
        Some("cs") => Some(SizeCategory::CsharpProduction),
        Some("md") => Some(SizeCategory::Markdown),
        _ => None,
    }
}

fn is_test_path(path: &Path, name: &str, suffixes: &[&str]) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some("test" | "tests" | "benches" | "examples")
        )
    }) || suffixes.iter().any(|suffix| name.ends_with(suffix))
}

fn ignored_directory(relative: &Path, policy: &Policy) -> bool {
    relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| policy.ignored_directories.contains(name))
        || ignored_prefix(relative, policy)
}

fn ignored_prefix(relative: &Path, policy: &Policy) -> bool {
    let text = relative.to_string_lossy().replace('\\', "/");
    policy
        .ignored_path_prefixes
        .iter()
        .any(|prefix| text == *prefix || text.starts_with(&format!("{prefix}/")))
}

pub(crate) fn relative_text(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{language_findings, size_category};
    use crate::config::SizeCategory;

    #[test]
    fn distinguishes_source_and_test_size_categories() {
        assert_eq!(
            size_category(Path::new("crates/core/src/lib.rs")),
            Some(SizeCategory::RustProduction)
        );
        assert_eq!(
            size_category(Path::new("crates/core/tests/state.rs")),
            Some(SizeCategory::RustTest)
        );
        assert_eq!(
            size_category(Path::new("managed/ModEntry.cs")),
            Some(SizeCategory::CsharpProduction)
        );
        assert_eq!(
            size_category(Path::new(".github/workflows/ci.yml")),
            Some(SizeCategory::Workflow)
        );
    }

    #[test]
    fn rejects_python_sources_and_manifests() {
        let root = Path::new("repository");
        let files = [
            PathBuf::from("repository/scripts/check.py"),
            PathBuf::from("repository/scripts/__pycache__/check.pyc"),
            PathBuf::from("repository/pyproject.toml"),
            PathBuf::from("repository/src/lib.rs"),
        ];
        let findings = language_findings(root, &files);
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn repository_policy_scans_the_runtime_binary_source() -> Result<(), String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let policy = crate::config::Policy::load(&root.join("policy.toml"))?;
        let files = super::collect(&root, &policy)?;
        for relative in [
            "crates/gateway/src/bin/sts2-gateway-runtime.rs",
            "crates/gateway/src/bin/runtime_support/service.rs",
            "crates/gateway/src/bin/runtime_support/service_config.rs",
            "crates/gateway/src/bin/runtime_support/service_lease.rs",
            "crates/gateway/src/bin/runtime_support/service_v2.rs",
            "crates/gateway/src/bin/runtime_support/service_routes_tests.rs",
            "crates/gateway/src/bin/runtime_support/http.rs",
            "crates/gateway/src/bin/runtime_support/http_deadline.rs",
            "crates/gateway/src/bin/runtime_support/http_tests.rs",
        ] {
            assert!(
                files.contains(&root.join(relative)),
                "policy omitted {relative}"
            );
        }
        Ok(())
    }
}
