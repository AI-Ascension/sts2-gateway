// SPDX-License-Identifier: MIT

use std::path::{Component, Path};

use toml::Value;

use crate::config::Policy;
use crate::diagnostic::Finding;

pub(super) fn path_array(
    value: Option<&Value>,
    key: &str,
    findings: &mut Vec<Finding>,
) -> Vec<String> {
    match value {
        None => {
            if key == "workspace.members" {
                findings.push(Finding::error(
                    "RUST001",
                    "Cargo.toml",
                    "workspace.members is missing",
                ));
            }
            Vec::new()
        }
        Some(Value::Array(values)) => values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                if let Some(value) = value.as_str() {
                    Some(value.to_owned())
                } else {
                    findings.push(Finding::error(
                        "RUST001",
                        "Cargo.toml",
                        format!("{key}[{index}] must be a string"),
                    ));
                    None
                }
            })
            .collect(),
        Some(_) => {
            findings.push(Finding::error(
                "RUST001",
                "Cargo.toml",
                format!("{key} must be an array"),
            ));
            Vec::new()
        }
    }
}

pub(super) fn matches_path(path: &str, pattern: &str) -> bool {
    let path = path.trim_matches('/');
    let pattern = pattern.trim_matches('/');
    if pattern == "." {
        return path.is_empty();
    }
    let path_parts: Vec<_> = path.split('/').collect();
    let pattern_parts: Vec<_> = pattern.split('/').collect();
    path_parts.len() == pattern_parts.len()
        && path_parts
            .iter()
            .zip(pattern_parts)
            .all(|(path, pattern)| pattern == "*" || wildcard_segment(path, pattern))
}

fn wildcard_segment(value: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return value == pattern;
    }
    // Cargo workspace globs use `*` within one path component.  Keep the
    // component boundary in `matches_path`, then match every star here so a
    // pattern such as `foo*bar*baz` cannot silently ignore its middle term.
    let value = value.as_bytes();
    let pattern = pattern.as_bytes();
    let (mut value_index, mut pattern_index) = (0, 0);
    let (mut star, mut retry) = (None, 0);
    while value_index < value.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == value[value_index] {
            value_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            retry = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            retry += 1;
            value_index = retry;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

pub(super) fn is_excluded(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let mut candidate = Some(path);
        while let Some(value) = candidate {
            if matches_path(value, pattern) {
                return true;
            }
            candidate = value.rsplit_once('/').map(|(parent, _)| parent);
        }
        !pattern.contains('*') && (path == pattern || path.starts_with(&format!("{pattern}/")))
    })
}

pub(super) fn ignored_prefix(path: &str, policy: &Policy) -> bool {
    policy
        .ignored_path_prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
}

pub(super) fn is_rust_binary_sources_dir(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "bin")
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "src")
}

pub(super) fn is_safe_relative_path(path: &str) -> bool {
    let value = Path::new(path);
    !value.is_absolute()
        && value
            .components()
            .all(|component| !matches!(component, Component::ParentDir))
}

pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{is_excluded, matches_path};

    #[test]
    fn matches_every_wildcard_in_a_component() {
        assert!(matches_path("foo-middle-baz", "foo*middle*baz"));
        assert!(!matches_path("foo-middle-nope", "foo*middle*baz"));
        assert!(!matches_path("nested/foo-middle-baz", "foo*middle*baz"));
    }

    #[test]
    fn excludes_nested_paths_without_hiding_sibling_components() {
        let patterns = vec![String::from("standards")];
        assert!(is_excluded("standards/tools/standards-sync", &patterns));
        assert!(!is_excluded("standardized/tools", &patterns));
    }
}
