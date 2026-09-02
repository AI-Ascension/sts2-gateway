// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files
        .iter()
        .filter(|path| path.extension().is_some_and(|value| value == "md"))
    {
        let relative = relative_text(root, path);
        match fs::read_to_string(path) {
            Ok(text) => check_links(path, &relative, &text, &mut findings),
            Err(error) => findings.push(Finding::error(
                "DOC002",
                &relative,
                format!("cannot read Markdown: {error}"),
            )),
        }
    }
    findings
}

fn check_links(path: &Path, relative: &str, text: &str, findings: &mut Vec<Finding>) {
    for target in link_targets(text) {
        let target = target.trim().trim_matches(['<', '>']);
        let local = target.split('#').next().unwrap_or_default();
        if local.is_empty()
            || local.starts_with("http://")
            || local.starts_with("https://")
            || local.starts_with("mailto:")
        {
            continue;
        }
        let resolved = path.parent().unwrap_or_else(|| Path::new("")).join(local);
        if !resolved.exists() {
            findings.push(Finding::error(
                "DOC002",
                relative,
                format!("local Markdown link target does not exist: {local}"),
            ));
        }
    }
}

fn link_targets(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("](").filter_map(|(start, _)| {
        let target = &text[start + 2..];
        let end = target.find(')')?;
        Some(&target[..end])
    })
}

#[cfg(test)]
mod tests {
    use super::link_targets;

    #[test]
    fn extracts_inline_markdown_targets() {
        let targets: Vec<_> =
            link_targets("[one](docs/one.md) and [two](https://example.test)").collect();
        assert_eq!(targets, ["docs/one.md", "https://example.test"]);
    }
}
