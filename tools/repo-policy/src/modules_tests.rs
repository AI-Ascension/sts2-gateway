// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use super::findings;

/// A throwaway package tree. Dropping it removes the directory, so a failing
/// assertion still cleans up.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Result<Self, String> {
        let root =
            std::env::temp_dir().join(format!("repo-policy-modules-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        Ok(Self { root })
    }

    fn write(&self, relative: &str, contents: &str) -> Result<(), String> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, contents).map_err(|error| error.to_string())
    }

    fn files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
                let path = entry.map_err(|error| error.to_string())?.path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        Ok(files)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn flags_a_module_that_no_root_reaches() -> Result<(), String> {
    let scratch = Scratch::new("orphan")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod live;\n")?;
    scratch.write("src/live.rs", "pub fn live() {}\n")?;
    scratch.write("src/orphan.rs", "pub fn orphan() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1, "expected exactly the orphan");
    assert_eq!(found[0].rule, "RUST002");
    assert_eq!(found[0].path, "src/orphan.rs");
    Ok(())
}

#[test]
fn accepts_path_siblings_mod_rs_children_and_target_roots() -> Result<(), String> {
    let scratch = Scratch::new("reachable")?;
    scratch.write(
        "Cargo.toml",
        "[package]\nname = \"scratch\"\n\n[[bin]]\nname = \"tool\"\npath = \"src/tool.rs\"\n",
    )?;
    scratch.write("src/lib.rs", "mod config;\nmod nested;\nmod parts;\n")?;
    scratch.write("src/config.rs", "#[path = \"sibling.rs\"]\nmod sibling;\n")?;
    scratch.write("src/sibling.rs", "pub fn sibling() {}\n")?;
    scratch.write("src/nested/mod.rs", "mod child;\n")?;
    scratch.write("src/nested/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/parts.rs", "include!(\"included/leaf.rs\");\n")?;
    scratch.write("src/included/leaf.rs", "mod deeper;\n")?;
    scratch.write("src/included/deeper.rs", "pub fn deeper() {}\n")?;
    scratch.write("src/tool.rs", "fn main() {}\n")?;
    scratch.write("src/bin/extra/main.rs", "mod helper;\nfn main() {}\n")?;
    scratch.write("src/bin/extra/helper.rs", "pub fn helper() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}

#[test]
fn follows_included_files_into_their_own_directory() -> Result<(), String> {
    let scratch = Scratch::new("include")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod parts;\n")?;
    scratch.write("src/parts.rs", "include!(\"included/leaf.rs\");\n")?;
    scratch.write("src/included/leaf.rs", "mod deeper;\n")?;
    scratch.write("src/included/deeper.rs", "pub fn deeper() {}\n")?;
    // A file beside the included leaf is not named by any `mod`, so it is an orphan.
    scratch.write("src/included/stray.rs", "pub fn stray() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, "src/included/stray.rs");
    Ok(())
}

#[test]
fn reaches_an_auto_discovered_bin_root() -> Result<(), String> {
    let scratch = Scratch::new("bin")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/bin/standalone.rs", "fn main() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}

/// The repository's own controls: after the deleted orphan, the real tree must
/// be green, and the files exercising each resolution rule must be present so
/// the green result is not vacuous.
#[test]
fn the_repository_tree_is_green() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let policy = crate::config::Policy::load(&root.join("policy.toml"))?;
    let files = crate::files::collect(&root, &policy)?;
    for control in [
        "crates/gateway/src/bin/sts2-gateway-runtime.rs",
        "crates/gateway/src/bin/runtime_support/service_config.rs",
        "crates/gateway/src/bin/runtime_support/service_config_runtime_v2.rs",
        "crates/gateway/src/bin/runtime_support/service_config_coop_native.rs",
        "crates/gateway/src/bin/runtime_support/service_config_values.rs",
        "crates/gateway/src/bin/runtime_support/service_test_modules.rs",
        "crates/gateway/src/runtime_v2/contract_types.rs",
        "crates/gateway/src/seeded_run/ledger.rs",
        "crates/gateway/tests/support/mod.rs",
    ] {
        assert!(
            files.contains(&root.join(control)),
            "control file missing from the tree: {control}"
        );
    }
    assert!(
        !root
            .join("crates/gateway/src/bin/runtime_support/service_config_identity.rs")
            .exists(),
        "the orphan must be deleted"
    );
    let found = findings(&root, &files);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}
