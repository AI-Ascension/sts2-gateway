// SPDX-License-Identifier: MIT

//! Tests for the reachability check.
//!
//! Module resolution depends on what exists on disk, so each case builds a real
//! crate in a temporary directory rather than mocking the filesystem.

use std::fs;
use std::path::{Path, PathBuf};

use super::findings;
use super::syntax::{attribute_text, path_attributes, strip_comments};

/// A crate on disk, since module resolution depends on what actually exists.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!("repo-policy-reach-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n")
            .map_err(|error| error.to_string())?;
        Ok(Self { root })
    }

    fn file(self, relative: &str, contents: &str) -> Result<Self, String> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, contents).map_err(|error| error.to_string())?;
        Ok(self)
    }

    fn unreachable(&self) -> Vec<String> {
        findings(&self.root)
            .into_iter()
            .map(|finding| finding.path)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_declared_module_is_reachable() -> Result<(), String> {
    let fixture = Fixture::new("declared")?
        .file("src/lib.rs", "mod kept;\n")?
        .file("src/kept.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn a_module_whose_declaration_was_lost_is_reported() -> Result<(), String> {
    let fixture = Fixture::new("lost")?
        .file("src/lib.rs", "")?
        .file("src/orphan.rs", "pub fn live() {}\n")?;
    assert_eq!(fixture.unreachable(), ["src/orphan.rs"]);
    Ok(())
}

#[test]
fn comments_do_not_declare_modules() -> Result<(), String> {
    let fixture = Fixture::new("comments")?
        .file("src/lib.rs", "// mod lost;\n\n/* mod also_lost; */\n")?
        .file("src/lost.rs", "")?;
    assert_eq!(fixture.unreachable(), ["src/lost.rs"]);
    Ok(())
}

#[test]
fn path_attribute_modules_and_their_children_are_reachable() -> Result<(), String> {
    let fixture = Fixture::new("path-attr")?
        .file(
            "src/lib.rs",
            "#[path = \"store/handoff.rs\"]\nmod handoff;\n",
        )?
        .file("src/store/handoff.rs", "mod schema;\n")?
        .file("src/store/schema.rs", "")?;
    // The child resolves beside the `#[path]`-loaded file, not under a directory
    // named after it.
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn a_path_attribute_before_a_visibility_is_honoured() -> Result<(), String> {
    // Regression: attributes precede the visibility, so `#[path = ...]` followed
    // by `pub(super) mod x;` must still be read. Scanning back from the `mod`
    // keyword alone treats the file and its children as orphans.
    let fixture = Fixture::new("path-attr-visibility")?
        .file(
            "src/lib.rs",
            "#[path = \"group/entry.rs\"]\npub(super) mod entry;\n",
        )?
        .file("src/group/entry.rs", "mod member;\n")?
        .file("src/group/member.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn an_inline_module_after_a_visibility_is_honoured() -> Result<(), String> {
    let fixture = Fixture::new("inline-visibility")?
        .file("src/lib.rs", "pub(crate) mod outer { mod inner; }\n")?
        .file("src/outer/inner.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn both_cfg_attr_branches_of_a_path_module_are_followed() -> Result<(), String> {
    let fixture = Fixture::new("cfg-attr")?
        .file(
            "src/lib.rs",
            "#[cfg_attr(unix, path = \"unix_snapshot.rs\")]\n\
             #[cfg_attr(not(unix), path = \"other_snapshot.rs\")]\n\
             mod snapshot;\n",
        )?
        .file("src/unix_snapshot.rs", "")?
        .file("src/other_snapshot.rs", "")?;
    assert_eq!(
        fixture.unreachable(),
        Vec::<String>::new(),
        "both platform branches must count as reachable"
    );
    Ok(())
}

#[test]
fn raw_identifier_modules_resolve_to_their_stem() -> Result<(), String> {
    let fixture = Fixture::new("raw-identifier")?
        .file("src/lib.rs", "mod r#move;\n")?
        .file("src/move.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn inline_modules_scope_their_children() -> Result<(), String> {
    let fixture = Fixture::new("inline")?
        .file("src/lib.rs", "mod outer { mod inner; }\n")?
        .file("src/outer/inner.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn include_targets_keep_the_includers_child_directory() -> Result<(), String> {
    let fixture = Fixture::new("include")?
        .file("src/lib.rs", "include!(\"generated.rs\");\n")?
        .file("src/generated.rs", "mod nested;\n")?
        .file("src/nested.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn every_cfg_attr_branch_is_collected_from_the_attribute_block() {
    let source = "#[cfg_attr(unix, path = \"snapshot.rs\")]\n\
                  #[cfg_attr(not(unix), path = \"other.rs\")]\nmod snapshot;\n";
    let at = source.find("mod").unwrap_or(0);
    assert_eq!(path_attributes(&attribute_text(source, at)).len(), 2);
}

#[test]
fn block_comments_are_ignored_but_code_after_them_survives() {
    let text = strip_comments("/* mod gone; */\nmod kept;\n");
    assert!(text.contains("mod kept;"));
    assert!(!text.contains("mod gone;"));
}

#[test]
fn unrelated_rust_files_outside_a_crate_are_not_reported() -> Result<(), String> {
    let fixture = Fixture::new("outside")?
        .file("src/lib.rs", "")?
        // `scratch/` is not a crate source directory, so it is out of scope.
        .file("scratch/loose.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn a_file_without_a_crate_root_is_not_judged() -> Result<(), String> {
    let fixture = Fixture::new("no-root")?.file("notes/anything.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn path_is_reported_relative_to_the_repository_root() -> Result<(), String> {
    let fixture = Fixture::new("relative")?
        .file("src/lib.rs", "")?
        .file("src/deep/orphan.rs", "")?;
    assert_eq!(fixture.unreachable(), ["src/deep/orphan.rs"]);
    Ok(())
}

#[test]
fn the_reported_message_names_the_repair() -> Result<(), String> {
    let fixture = Fixture::new("message")?
        .file("src/lib.rs", "")?
        .file("src/orphan.rs", "")?;
    let reported = findings(&fixture.root);
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].rule, "RUST002");
    assert!(
        reported[0].message.contains("mod"),
        "{}",
        reported[0].message
    );
    Ok(())
}

#[test]
fn a_repository_with_no_cargo_manifest_is_ignored() -> Result<(), String> {
    let root = std::env::temp_dir().join("repo-policy-reach-empty");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).map_err(|error| error.to_string())?;
    fs::write(root.join("src/loose.rs"), "").map_err(|error| error.to_string())?;
    assert_eq!(findings(&root).len(), 0);
    let _ = fs::remove_dir_all(&root);
    Ok(())
}

#[test]
fn declaration_stem_is_matched_across_paths() -> Result<(), String> {
    // Guards word-boundary handling: `mod` inside another identifier is not a
    // declaration.
    let fixture = Fixture::new("boundary")?
        .file("src/lib.rs", "fn modify() {}\n")?
        .file("src/if.rs", "")?;
    assert_eq!(fixture.unreachable(), ["src/if.rs"]);
    Ok(())
}

#[test]
fn directory_module_files_are_reachable() -> Result<(), String> {
    let fixture = Fixture::new("directory-module")?
        .file("src/lib.rs", "mod group;\n")?
        .file("src/group/mod.rs", "mod member;\n")?
        .file("src/group/member.rs", "")?;
    assert_eq!(fixture.unreachable(), Vec::<String>::new());
    Ok(())
}

#[test]
fn a_reported_path_is_relative_to_the_root() -> Result<(), String> {
    let fixture = Fixture::new("root-child")?
        .file("src/lib.rs", "")?
        .file("src/orphan.rs", "")?;
    let reported = fixture.unreachable();
    assert!(Path::new(&reported[0]).is_relative(), "{reported:?}");
    Ok(())
}
