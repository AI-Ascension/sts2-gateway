// SPDX-License-Identifier: MIT

//! Rust module-reachability policy.
//!
//! Rust compiles only the source files a crate root reaches through `mod`,
//! `#[path]`, or `include!`. A lost declaration therefore drops a live file
//! from the build silently: it is never compiled, so no compiler or test ever
//! reports it. This check mirrors rustc's own filename resolution and flags
//! every `.rs` file in a compiled package that no crate root reaches.
//!
//! Resolution rules implemented here (verified against `rustc` 1.97.1):
//! - roots are `[lib].path`/`src/lib.rs`, `[[bin]].path`/`src/main.rs`/
//!   `src/bin/*.rs`/`src/bin/*/main.rs`, and the `tests/`, `examples/`, and
//!   `benches/` target conventions;
//! - a plain `mod x;` resolves to `DIR/x.rs` or `DIR/x/mod.rs`, where `DIR` is
//!   the crate root or `mod.rs` directory, and a `STEM/` subdirectory for any
//!   other file;
//! - a `#[path = "..."]`-loaded file (and its `#[cfg_attr]` variant) keeps its
//!   children in its own directory, named relative to the file that loads it;
//! - an `include!`-ed file is compiled in place and keeps its children in its
//!   own directory;
//! - an inline `mod x { ... }` block nests its file children one level deeper.

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::diagnostic::Finding;
use crate::rust_source::{declarations, include_paths};

const RULE: &str = "RUST002";

/// A file module declaration: `mod NAME;` under some number of inline blocks,
/// with any `#[path]` values and whether the path came from a `cfg_attr`.
pub(crate) struct Declaration {
    pub(crate) name: String,
    pub(crate) inline: Vec<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) conditional: bool,
}

/// Reports every `.rs` file under a compiled package that no crate root reaches.
pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut packages = Vec::new();
    let mut reachable = BTreeSet::new();
    for manifest in files.iter().filter(|file| is_named(file, "Cargo.toml")) {
        let Ok(text) = fs::read_to_string(manifest) else {
            continue;
        };
        let Ok(value) = toml::from_str::<Value>(&text) else {
            continue;
        };
        if value.get("package").and_then(Value::as_table).is_none() {
            continue;
        }
        let Some(directory) = manifest.parent() else {
            continue;
        };
        packages.push(directory.to_path_buf());
        reach(directory, &target_roots(directory, &value), &mut reachable);
    }
    files
        .iter()
        .filter(|file| {
            file.extension().and_then(|value| value.to_str()) == Some("rs")
                && !reachable.contains(*file)
                && packages.iter().any(|package| file.starts_with(package))
        })
        .map(|file| {
            Finding::error(
                RULE,
                &crate::files::relative_text(root, file),
                "no crate root reaches this module file; restore its `mod`/`#[path]`/`include!` \
                 declaration or delete it",
            )
        })
        .collect()
}

fn is_named(file: &Path, name: &str) -> bool {
    file.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == name)
}

/// Crate roots: `[lib]`, every `[[bin]]`/`[[test]]`/`[[example]]`/`[[bench]]`
/// path, and the auto-discovered files Cargo treats as targets.
fn target_roots(directory: &Path, manifest: &Value) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::new();
    match manifest
        .get("lib")
        .and_then(|value| value.get("path"))
        .and_then(Value::as_str)
    {
        Some(path) => add_root(&mut roots, directory, path),
        None => add_root(&mut roots, directory, "src/lib.rs"),
    }
    for (key, conventional) in [
        ("bin", "src/bin"),
        ("test", "tests"),
        ("example", "examples"),
        ("bench", "benches"),
    ] {
        for entry in manifest
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(path) = entry.get("path").and_then(Value::as_str) {
                add_root(&mut roots, directory, path);
            } else if let Some(name) = entry.get("name").and_then(Value::as_str) {
                add_root(&mut roots, directory, &format!("{conventional}/{name}.rs"));
            }
        }
    }
    add_root(&mut roots, directory, "src/main.rs");
    for conventional in ["src/bin", "tests", "examples", "benches"] {
        let Ok(entries) = fs::read_dir(directory.join(conventional)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                roots.insert(path);
            } else if path.join("main.rs").is_file() {
                roots.insert(path.join("main.rs"));
            }
        }
    }
    roots
}

fn add_root(roots: &mut BTreeSet<PathBuf>, directory: &Path, relative: &str) {
    let path = directory.join(relative);
    if path.is_file() {
        roots.insert(path);
    }
}

/// Walks outward from every root, resolving each declaration and noting every
/// file reached. Pairs are visited once so a file loaded two ways merges both
/// directory scopes instead of looping.
fn reach(package: &Path, roots: &BTreeSet<PathBuf>, reachable: &mut BTreeSet<PathBuf>) {
    let mut queue: VecDeque<(PathBuf, PathBuf)> = VecDeque::new();
    let mut visited: BTreeSet<(PathBuf, PathBuf)> = BTreeSet::new();
    for root in roots {
        queue.push_back((root.clone(), root.parent().unwrap_or(package).to_path_buf()));
    }
    while let Some((file, module_dir)) = queue.pop_front() {
        if !visited.insert((file.clone(), module_dir.clone())) {
            continue;
        }
        reachable.insert(file.clone());
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let file_dir = file.parent().unwrap_or(package).to_path_buf();
        for declaration in declarations(&text) {
            let mut base = module_dir.clone();
            for name in &declaration.inline {
                base.push(name);
            }
            resolve(&declaration, &base, &file_dir, &mut queue);
        }
        for target in include_paths(&text) {
            let included = file_dir.join(target);
            if included.is_file() {
                let directory = included.parent().unwrap_or(package).to_path_buf();
                queue.push_back((included, directory));
            }
        }
    }
}

fn resolve(
    declaration: &Declaration,
    base: &Path,
    file_dir: &Path,
    queue: &mut VecDeque<(PathBuf, PathBuf)>,
) {
    for path in &declaration.paths {
        let target = file_dir.join(path);
        if target.is_file() {
            queue.push_back((
                target.clone(),
                target.parent().unwrap_or(base).to_path_buf(),
            ));
        }
    }
    if !declaration.paths.is_empty() && !declaration.conditional {
        return;
    }
    let nested = base.join(&declaration.name);
    let direct = base.join(format!("{}.rs", declaration.name));
    if direct.is_file() {
        queue.push_back((direct, nested));
    } else {
        let module = nested.join("mod.rs");
        if module.is_file() {
            queue.push_back((module, nested));
        }
    }
}

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
