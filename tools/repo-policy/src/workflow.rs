// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files.iter().filter(|path| is_workflow(root, path)) {
        let relative = relative_text(root, path);
        match fs::read_to_string(path) {
            Ok(text) => check(&relative, &text, &mut findings),
            Err(error) => findings.push(Finding::error(
                "WF001",
                &relative,
                format!("cannot read workflow: {error}"),
            )),
        }
    }
    findings
}

fn check(relative: &str, text: &str, findings: &mut Vec<Finding>) {
    if !text.lines().any(|line| line.starts_with("permissions:")) {
        findings.push(Finding::error(
            "WF001",
            relative,
            "missing explicit top-level permissions",
        ));
    }
    if text
        .lines()
        .any(|line| line.trim_start().starts_with("pull_request_target:"))
    {
        findings.push(Finding::error(
            "WF002",
            relative,
            "pull_request_target is prohibited",
        ));
    }
    if text
        .lines()
        .any(|line| line.trim().starts_with("continue-on-error: true"))
    {
        findings.push(Finding::error(
            "WF003",
            relative,
            "continue-on-error: true is prohibited",
        ));
    }
    if text.lines().any(|line| line.contains("|| true")) {
        findings.push(Finding::error(
            "WF005",
            relative,
            "unconditional shell success with `|| true` is prohibited",
        ));
    }
    for reference in action_references(text) {
        if !immutable_action_reference(reference) {
            findings.push(Finding::error(
                "WF004",
                relative,
                format!("action is not pinned to an immutable digest: {reference}"),
            ));
        }
    }
}

fn action_references(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter_map(|line| {
        let trimmed = line
            .trim_start()
            .strip_prefix('-')
            .map_or_else(|| line.trim_start(), str::trim_start);
        let value = trimmed.strip_prefix("uses:")?.trim();
        let without_comment = value.split('#').next()?.trim();
        Some(without_comment.trim_matches(['\'', '"']))
    })
}

fn immutable_action_reference(reference: &str) -> bool {
    if reference.starts_with("./") {
        return true;
    }
    if let Some(digest) = reference
        .strip_prefix("docker://")
        .and_then(|value| value.rsplit_once("@sha256:").map(|pair| pair.1))
    {
        return digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    reference.rsplit_once('@').is_some_and(|(_, revision)| {
        revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn is_workflow(root: &Path, path: &Path) -> bool {
    relative_text(root, path).starts_with(".github/workflows/")
        && matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        )
}

#[cfg(test)]
mod tests {
    use super::{action_references, immutable_action_reference};

    #[test]
    fn accepts_only_local_or_immutable_action_references() {
        assert!(immutable_action_reference("./.github/actions/local"));
        assert!(immutable_action_reference(
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
        ));
        assert!(!immutable_action_reference("actions/checkout@v7"));
    }

    #[test]
    fn extracts_action_references_without_comments_or_quotes() {
        let text = "    - uses: 'actions/checkout@0123456789012345678901234567890123456789' # v1\n";
        let references: Vec<_> = action_references(text).collect();
        assert_eq!(
            references,
            ["actions/checkout@0123456789012345678901234567890123456789"]
        );
    }
}
