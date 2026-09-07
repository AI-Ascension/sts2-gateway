// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use toml::Value;

const LEGACY_POLICY_VERSION: i64 = 1;
const CURRENT_POLICY_VERSION: i64 = 2;
const MANAGED_STANDARDS_PATH: &str = "standards/tools/standards-sync";
const KNOWN_RULES: &[&str] = &[
    "BOUND001", "CFG001", "DOC001", "DOC002", "DOC003", "EXC001", "LANG001", "LIC001", "LIC002",
    "LIC003", "RUST001", "RUST002", "RUST003", "RUST004", "RUST005", "SIZE001", "WF001", "WF002",
    "WF003", "WF004", "WF005",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SizeCategory {
    RustProduction,
    RustTest,
    CsharpProduction,
    CsharpTest,
    Workflow,
    Markdown,
}

impl SizeCategory {
    fn key(self) -> &'static str {
        match self {
            Self::RustProduction => "rust_production",
            Self::RustTest => "rust_test",
            Self::CsharpProduction => "csharp_production",
            Self::CsharpTest => "csharp_test",
            Self::Workflow => "workflow",
            Self::Markdown => "markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Budget {
    pub(crate) preferred: usize,
    pub(crate) maximum: usize,
}

#[derive(Debug)]
pub(crate) struct Policy {
    pub(crate) policy_version: i64,
    pub(crate) advisory_rules: BTreeSet<String>,
    pub(crate) required_files: Vec<String>,
    pub(crate) ignored_directories: BTreeSet<String>,
    pub(crate) ignored_path_prefixes: BTreeSet<String>,
    pub(crate) exemptions: BTreeMap<String, String>,
    limits: BTreeMap<&'static str, Budget>,
}

impl Policy {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let value = toml::from_str::<Value>(text)
            .map_err(|error| format!("cannot parse policy.toml: {error}"))?;
        let root = value
            .as_table()
            .ok_or_else(|| "policy.toml root must be a table".to_owned())?;
        let version = root.get("policy_version").and_then(Value::as_integer);
        let policy_version =
            version.ok_or_else(|| "policy_version must be an integer".to_owned())?;
        if !matches!(
            policy_version,
            LEGACY_POLICY_VERSION | CURRENT_POLICY_VERSION
        ) {
            return Err(format!(
                "policy_version must be {LEGACY_POLICY_VERSION} or {CURRENT_POLICY_VERSION}, found {policy_version}"
            ));
        }

        let project = table(root.get("project"), "project")?;
        let limits = table(root.get("limits"), "limits")?;
        let exemptions = table(root.get("exemptions"), "exemptions")?;
        let advisory_rules = parse_severity(root, policy_version)?;
        let required_files = string_array(project.get("required_files"), "required_files")?;
        validate_exact_paths(&required_files, "required_files")?;
        let exemptions = string_table(exemptions, "exemptions")?;
        for path in exemptions.keys() {
            validate_exact_path(path, "exemptions")?;
        }
        let ignored_directories = validate_ignored_directories(&string_array(
            project.get("ignored_directories"),
            "ignored_directories",
        )?)?;
        let ignored_path_prefixes = validate_ignored_path_prefixes(&string_array(
            project.get("ignored_path_prefixes"),
            "ignored_path_prefixes",
        )?)?;
        Ok(Self {
            policy_version,
            advisory_rules,
            required_files,
            ignored_directories,
            ignored_path_prefixes,
            exemptions,
            limits: parse_limits(limits)?,
        })
    }

    pub(crate) fn budget(&self, category: SizeCategory) -> Budget {
        self.limits[category.key()]
    }
}

fn validate_exact_paths(values: &[String], key: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_exact_path(value, key)?;
        if !seen.insert(value) {
            return Err(format!("{key} contains duplicate path {value}"));
        }
    }
    Ok(())
}

fn validate_ignored_directories(values: &[String]) -> Result<BTreeSet<String>, String> {
    let mut directories = BTreeSet::new();
    for directory in values {
        let path = Path::new(directory);
        let safe = !directory.is_empty()
            && !directory.contains('\\')
            && !directory.contains('/')
            && !directory.contains('*')
            && !directory.contains('?')
            && path.components().count() == 1
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            && !directory.eq_ignore_ascii_case("standards");
        if !safe {
            return Err(format!(
                "ignored_directories must contain one safe directory name and cannot hide standards: {directory}"
            ));
        }
        if !directories.insert(directory.clone()) {
            return Err(format!(
                "ignored_directories contains duplicate directory {directory}"
            ));
        }
    }
    Ok(directories)
}

fn validate_exact_path(value: &str, key: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains('*')
        || value.contains('?')
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{key} must contain exact repository-relative paths: {value}"
        ));
    }
    Ok(())
}

fn parse_severity(
    root: &toml::map::Map<String, Value>,
    policy_version: i64,
) -> Result<BTreeSet<String>, String> {
    if policy_version == LEGACY_POLICY_VERSION {
        return Ok(BTreeSet::new());
    }
    let severity = table(root.get("severity"), "severity")?;
    let mandatory = string_array(severity.get("mandatory"), "severity.mandatory")?;
    validate_rule_list(&mandatory, "severity.mandatory", true)?;
    if !mandatory.iter().any(|rule| rule == "*") {
        return Err(
            "severity.mandatory must include \"*\" as the default classification".to_owned(),
        );
    }
    let advisory_values = string_array(severity.get("advisory"), "severity.advisory")?;
    validate_rule_list(&advisory_values, "severity.advisory", false)?;
    let advisory: BTreeSet<String> = advisory_values.into_iter().collect();
    if advisory.is_empty() {
        return Err("severity.advisory must name at least one rule".to_owned());
    }
    if advisory
        .iter()
        .any(|rule| mandatory.iter().any(|item| item == rule))
    {
        return Err("severity rules cannot be both mandatory and advisory".to_owned());
    }
    Ok(advisory)
}

fn validate_rule_list(values: &[String], key: &str, allow_default: bool) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for rule in values {
        if !seen.insert(rule.as_str()) {
            return Err(format!("{key} contains duplicate rule {rule}"));
        }
        if rule == "*" {
            if !allow_default {
                return Err(format!("{key} cannot contain the default wildcard"));
            }
        } else if !KNOWN_RULES.contains(&rule.as_str()) {
            return Err(format!("{key} contains unknown rule {rule}"));
        } else if !allow_default && rule != "SIZE001" {
            return Err(format!("{key} cannot demote mandatory rule {rule}"));
        }
    }
    Ok(())
}

fn validate_ignored_path_prefixes(values: &[String]) -> Result<BTreeSet<String>, String> {
    let mut prefixes = BTreeSet::new();
    for prefix in values {
        let path = Path::new(prefix);
        let safe = !prefix.is_empty()
            && !prefix.contains('\\')
            && !prefix.starts_with('/')
            && !prefix.ends_with('/')
            && !prefix.contains('*')
            && !prefix.contains('?')
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !safe {
            return Err(format!(
                "ignored_path_prefixes must contain exact repository-relative paths: {prefix}"
            ));
        }
        if prefix == "standards"
            || prefix == "standards/tools"
            || MANAGED_STANDARDS_PATH.starts_with(&format!("{prefix}/"))
        {
            return Err(format!(
                "ignored_path_prefixes cannot hide the standards root; use the exact managed path {MANAGED_STANDARDS_PATH}"
            ));
        }
        if !prefixes.insert(prefix.clone()) {
            return Err(format!(
                "ignored_path_prefixes contains duplicate path {prefix}"
            ));
        }
    }
    Ok(prefixes)
}

fn parse_limits(
    table: &toml::map::Map<String, Value>,
) -> Result<BTreeMap<&'static str, Budget>, String> {
    let mut result = BTreeMap::new();
    for category in [
        SizeCategory::RustProduction,
        SizeCategory::RustTest,
        SizeCategory::CsharpProduction,
        SizeCategory::CsharpTest,
        SizeCategory::Workflow,
        SizeCategory::Markdown,
    ] {
        let key = category.key();
        let preferred = positive_usize(table.get(&format!("{key}_preferred")), key)?;
        let maximum = positive_usize(table.get(&format!("{key}_max")), key)?;
        if preferred > maximum {
            return Err(format!("{key}_preferred cannot exceed {key}_max"));
        }
        result.insert(key, Budget { preferred, maximum });
    }
    Ok(result)
}

fn table<'a>(
    value: Option<&'a Value>,
    key: &str,
) -> Result<&'a toml::map::Map<String, Value>, String> {
    value
        .and_then(Value::as_table)
        .ok_or_else(|| format!("{key} must be a table"))
}

fn string_array(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    let array = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array"))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn string_table(
    table: &toml::map::Map<String, Value>,
    key: &str,
) -> Result<BTreeMap<String, String>, String> {
    table
        .iter()
        .map(|(path, value)| {
            value
                .as_str()
                .map(|reason| (path.clone(), reason.to_owned()))
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn positive_usize(value: Option<&Value>, key: &str) -> Result<usize, String> {
    let integer = value
        .and_then(Value::as_integer)
        .ok_or_else(|| format!("{key} limit must be an integer"))?;
    usize::try_from(integer)
        .ok()
        .filter(|limit| *limit > 0)
        .ok_or_else(|| format!("{key} limit must be positive"))
}

#[cfg(test)]
mod tests {
    use super::{Policy, SizeCategory};

    const LIMITS: &str = r"
[limits]
rust_production_preferred = 10
rust_production_max = 20
rust_test_preferred = 11
rust_test_max = 21
csharp_production_preferred = 12
csharp_production_max = 22
csharp_test_preferred = 13
csharp_test_max = 23
workflow_preferred = 14
workflow_max = 24
markdown_preferred = 15
markdown_max = 25
";

    #[test]
    fn parses_complete_policy() -> Result<(), String> {
        let text = format!(
            "policy_version = 1\n[project]\nrequired_files = [\"README.md\"]\n\
             ignored_directories = [\"target\"]\nignored_path_prefixes = []\n\
             {LIMITS}\n[exemptions]\n"
        );
        let policy = Policy::parse(&text)?;
        assert_eq!(policy.required_files, ["README.md"]);
        assert_eq!(policy.budget(SizeCategory::RustProduction).maximum, 20);
        Ok(())
    }

    #[test]
    fn rejects_inverted_budget() {
        let text = format!(
            "policy_version = 1\n[project]\nrequired_files = []\n\
             ignored_directories = []\nignored_path_prefixes = []\n\
             {}\n[exemptions]\n",
            LIMITS.replace("rust_production_max = 20", "rust_production_max = 5")
        );
        assert!(Policy::parse(&text).is_err());
    }

    #[test]
    fn rejects_unknown_and_duplicate_severity_rules() {
        let base = format!(
            "policy_version = 2\n[project]\nrequired_files = []\nignored_directories = []\nignored_path_prefixes = []\n[severity]\nmandatory = [\"*\"]\nadvisory = [\"SIZE001\"]\n{LIMITS}\n[exemptions]\n"
        );
        assert!(
            Policy::parse(&base.replace("advisory = [\"SIZE001\"]", "advisory = [\"RUST999\"]"))
                .is_err()
        );
        assert!(
            Policy::parse(&base.replace("advisory = [\"SIZE001\"]", "advisory = [\"RUST001\"]"))
                .is_err()
        );
        assert!(
            Policy::parse(&base.replace("mandatory = [\"*\"]", "mandatory = [\"*\", \"*\"]"))
                .is_err()
        );
        assert!(
            Policy::parse(&base.replace(
                "advisory = [\"SIZE001\"]",
                "advisory = [\"SIZE001\", \"SIZE001\"]"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_broad_standards_ignore_but_accepts_exact_managed_path() {
        let base = format!(
            "policy_version = 2\n[project]\nrequired_files = []\nignored_directories = []\nignored_path_prefixes = [\"standards/tools/standards-sync\"]\n[severity]\nmandatory = [\"*\"]\nadvisory = [\"SIZE001\"]\n{LIMITS}\n[exemptions]\n"
        );
        assert!(Policy::parse(&base).is_ok());
        assert!(
            Policy::parse(&base.replace("standards/tools/standards-sync", "standards")).is_err()
        );
    }

    #[test]
    fn rejects_path_traversal_in_required_files() {
        let text = format!(
            "policy_version = 1\n[project]\nrequired_files = [\"../outside\"]\n\
             ignored_directories = []\nignored_path_prefixes = []\n\
             {LIMITS}\n[exemptions]\n"
        );
        assert!(Policy::parse(&text).is_err());
    }
}
