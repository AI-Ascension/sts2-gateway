// SPDX-License-Identifier: MIT

//! Reachability check for Rust module files.
//!
//! rustc compiles only source files reachable from a crate root. Losing a `mod`
//! declaration is not an error anywhere: the file simply stops being read, which
//! silently drops live code and any `#[test]` inside it. This check reports Rust
//! files under a crate's source directories that no crate root can reach, so a
//! lost declaration turns a policy run red instead of disappearing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

mod scan;
mod syntax;

use scan::reachable;

const SOURCE_DIRECTORIES: &[&str] = &["src", "tests", "benches", "examples"];
const IGNORED_DIRECTORIES: &[&str] = &[".git", "target", "node_modules", "vendor", "obj"];
const MESSAGE: &str = "Rust module is not reachable from any crate root; its declaration was lost. \
                       Restore the `mod` item or delete the file.";

pub(crate) fn findings(root: &Path) -> Vec<Finding> {
    let mut unreachable = BTreeSet::new();
    for crate_directory in crate_directories(root) {
        let modules = modules_of(&crate_directory);
        let reached = reachable(&crate_directory, &modules);
        unreachable.extend(modules.into_iter().filter(|path| !reached.contains(path)));
    }
    unreachable
        .into_iter()
        .map(|path| Finding::error("RUST002", &relative_text(root, &path), MESSAGE))
        .collect()
}

/// Directories holding a `Cargo.toml`, which mark where a crate begins.
fn crate_directories(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut has_manifest = false;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                if !IGNORED_DIRECTORIES.contains(&name.as_str()) {
                    pending.push(entry.path());
                }
            } else if name == "Cargo.toml" {
                has_manifest = true;
            }
        }
        if has_manifest {
            found.push(directory);
        }
    }
    found.sort();
    found
}

/// Every Rust file inside a crate's source directories.
fn modules_of(crate_directory: &Path) -> Vec<PathBuf> {
    let mut modules = BTreeSet::new();
    for source in SOURCE_DIRECTORIES {
        let directory = crate_directory.join(source);
        if !directory.is_dir() {
            continue;
        }
        let mut pending = vec![directory];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    pending.push(path);
                } else if path.extension().is_some_and(|value| value == "rs") {
                    modules.insert(path);
                }
            }
        }
    }
    modules.into_iter().collect()
}

#[cfg(test)]
mod tests;
