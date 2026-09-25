// SPDX-License-Identifier: MIT

//! Module-resolution model.
//!
//! Follows rustc's rules closely enough to decide reachability:
//!
//! * a crate root (or a manifest `path =`) keeps its own directory as the
//!   directory its `mod` children resolve in;
//! * `mod name;` in any other file resolves beside that file's own directory,
//!   in `<dir>/name.rs` or `<dir>/name/mod.rs`;
//! * `#[path = "..."]` resolves relative to the file carrying the attribute, and
//!   the loaded module's children resolve beside the loaded file;
//! * inline `mod name { ... }` contributes children in `<dir>/name/`;
//! * `include!` inserts text, so it shares the includer's child directory.
//!
//! `cfg` and `cfg_attr` gating is deliberately ignored: a module that is compiled
//! out on this platform is still reachable, and treating it as unreachable would
//! report false positives. Being conservative in this direction is required, since
//! a wrong "unreachable" finding would block every legitimate build.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::syntax::{declarations, strip_comments};

/// Files reachable from a crate's roots, following the rules above.
pub(crate) fn reachable(crate_directory: &Path, modules: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut reached = BTreeSet::new();
    let mut pending: Vec<(PathBuf, PathBuf)> = modules
        .iter()
        .filter(|path| is_crate_root(crate_directory, path))
        .map(|path| (path.clone(), parent_of(path)))
        .collect();
    pending.extend(manifest_targets(crate_directory));
    while let Some((file, directory)) = pending.pop() {
        if !reached.insert(file.clone()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let file_directory = parent_of(&file);
        for (target, child) in declarations(&strip_comments(&text), &directory, &file_directory) {
            if target.is_file() {
                pending.push((target, child));
            }
        }
    }
    reached
}

/// A crate root keeps its own directory as the child module directory.
fn is_crate_root(crate_directory: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(crate_directory) else {
        return false;
    };
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let names: Vec<&str> = parts.iter().map(String::as_str).collect();
    matches!(
        names.as_slice(),
        ["src", "lib.rs" | "main.rs"]
            | ["src", "bin", _]
            | ["src", "bin", _, "main.rs"]
            | ["tests" | "benches" | "examples", _]
    )
}

/// Targets declared with an explicit `path = "*.rs"` in the crate manifest.
fn manifest_targets(crate_directory: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Ok(text) = fs::read_to_string(crate_directory.join("Cargo.toml")) else {
        return Vec::new();
    };
    quoted_paths(&text)
        .into_iter()
        .map(|relative| crate_directory.join(relative))
        .filter(|path| path.is_file())
        .map(|path| {
            let directory = parent_of(&path);
            (path, directory)
        })
        .collect()
}

pub(super) fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Every `"*.rs"` value on a `path =` line of a Cargo manifest.
fn quoted_paths(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if value.ends_with(".rs") {
            found.push(value.to_owned());
        }
    }
    found
}
