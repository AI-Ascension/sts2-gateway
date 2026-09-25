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
//! - roots are `[lib].path`/`src/lib.rs`, `[[bin]].path`/`src/main.rs` and the
//!   `src/bin/`, `tests/`, `examples/`, and `benches/` target conventions;
//! - a plain `mod x;` resolves to `DIR/x.rs` or `DIR/x/mod.rs` (where `DIR` is
//!   the crate root or `mod.rs` directory, and a `STEM/` subdirectory for any
//!   other file);
//! - a `#[path = "..."]` file (and its `#[cfg_attr]` variant) keeps its children
//!   in its own directory; the value resolves against an enclosing inline
//!   block's `#[path]` directory, or else the carrying file's own;
//! - an `include!`-ed file is compiled in place, keeping its own directory;
//! - an inline block owns no file, nests its children one level deeper, and its
//!   own `#[path]`, if any, names their directory outright.

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

use crate::diagnostic::Finding;
use crate::rust_source::{declarations, include_paths};

const RULE: &str = "RUST002";

/// A module declaration: `mod NAME;` or `mod NAME { ... }` under some number of
/// inline blocks, with any `#[path]` values and whether one came from a
/// `cfg_attr`.
#[derive(Clone)]
pub(crate) struct Declaration {
    pub(crate) name: String,
    /// `true` for `mod NAME;`, `false` for `mod NAME { ... }`.
    pub(crate) semi: bool,
    pub(crate) inline: Vec<Inline>,
    pub(crate) paths: Vec<String>,
    pub(crate) conditional: bool,
}

/// An enclosing inline `mod NAME { ... }` block and the `#[path]` it was
/// written with: on an inline block that attribute names the directory its
/// children live in, so it is part of their resolution base, not a file.
#[derive(Clone)]
pub(crate) struct Inline {
    pub(crate) name: String,
    pub(crate) paths: Vec<String>,
    pub(crate) conditional: bool,
}

/// Where a declaration's children resolve, one entry per `#[cfg_attr]` branch.
struct Base {
    /// The directory the enclosing blocks have contributed so far.
    path: PathBuf,
    /// Whether any enclosing block contributed it. Until one does, a `#[path]`
    /// value is relative to the *file's* own directory (`src/` for `src/x.rs`,
    /// not the `src/x/` its ordinary children use).
    contributed: bool,
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
                && !reachable.contains(&normalise(file))
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
        reachable.insert(normalise(&file));
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let file_dir = file.parent().unwrap_or(package).to_path_buf();
        for declaration in declarations(&text) {
            for base in bases(&declaration, &module_dir, &file_dir) {
                resolve(&declaration, &base, &file_dir, &mut queue);
            }
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

/// Every resolution base for a declaration, one per `#[cfg_attr]` branch of
/// every enclosing inline block, outermost first.
///
/// A plain inline `mod name { ... }` nests its children one directory deeper.
/// A block carrying `#[path = "..."]` names its children's directory outright
/// (rustc reads no file there, so the block owns nothing itself) and the value
/// supersedes enclosing names, being resolved against the directory the file
/// itself would have contributed. `rustc` 1.97.1, markers in every candidate:
/// `mod a { #[path = "t"] pub mod b { pub mod child; } }` in `src/x.rs` compiles
/// `src/x/a/t/child.rs`, while the same block unnested in `src/x.rs` compiles
/// `src/t/child.rs` — the file's own directory, not its `x/` module directory,
/// which is why both terms exist rather than one.
fn bases(declaration: &Declaration, module_dir: &Path, file_dir: &Path) -> Vec<Base> {
    let mut states = vec![Base {
        path: module_dir.to_path_buf(),
        contributed: false,
    }];
    for block in &declaration.inline {
        states = states
            .into_iter()
            .flat_map(|state| {
                let by_name = Base {
                    path: state.path.join(&block.name),
                    contributed: true,
                };
                // `cfg`/`cfg_attr` gates are treated as always taken, so a path
                // that came from one keeps the name-based branch as well: only a
                // file reachable under *no* gate may be reported.
                let mut branches = Vec::new();
                if block.paths.is_empty() || block.conditional {
                    branches.push(by_name);
                }
                let base = if state.contributed {
                    state.path.as_path()
                } else {
                    file_dir
                };
                branches.extend(block.paths.iter().map(|path| Base {
                    path: normalise(&base.join(path)),
                    contributed: true,
                }));
                branches
            })
            .collect();
    }
    states
}

fn resolve(
    declaration: &Declaration,
    base: &Base,
    file_dir: &Path,
    queue: &mut VecDeque<(PathBuf, PathBuf)>,
) {
    // An inline block owns no file: `rustc` reads none for it, `#[path]` or
    // not, because the value names the *directory* its children live in.
    if !declaration.semi {
        return;
    }
    // A `#[path]` value is relative to the directory the enclosing blocks have
    // folded to, and to the carrying file's own directory until one has.
    let anchor = if base.contributed {
        base.path.as_path()
    } else {
        file_dir
    };
    for path in &declaration.paths {
        let target = normalise(&anchor.join(path));
        if target.is_file() {
            queue.push_back((
                target.clone(),
                target.parent().unwrap_or(file_dir).to_path_buf(),
            ));
        }
    }
    if !declaration.paths.is_empty() && !declaration.conditional {
        return;
    }
    let nested = base.path.join(&declaration.name);
    let direct = base.path.join(format!("{}.rs", declaration.name));
    if direct.is_file() {
        queue.push_back((direct, nested));
    } else {
        let module = nested.join("mod.rs");
        if module.is_file() {
            queue.push_back((module, nested));
        }
    }
}

/// Removes `.` and `..` components textually.
///
/// A `#[path = "../other/y.rs"]` value is authored relative to the file carrying
/// the attribute, so it routinely escapes its own directory. The filesystem
/// resolves such a path on `is_file`, but the unreduced spelling would then be
/// inserted into the reachable set while `files::collect` records the reduced
/// one, and the file would be reported as unreachable.
fn normalise(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            other => parts.push(other.as_os_str().to_owned()),
        }
    }
    let mut normalised = PathBuf::new();
    for part in parts {
        normalised.push(part);
    }
    normalised
}

#[cfg(test)]
#[path = "modules_tests.rs"]
mod tests;
