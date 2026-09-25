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

/// `mod r#move;` resolves through the ordinary name, so the file is
/// `move.rs` — never `r#move.rs`.
#[test]
fn resolves_a_raw_identifier_module_name() -> Result<(), String> {
    let scratch = Scratch::new("raw-ident")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod r#move;\n")?;
    scratch.write("src/move.rs", "pub fn moved() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}

/// An attribute group between the `#[path]` and the item must not hide the
/// `#[path]`, in either order.
#[test]
fn reaches_path_modules_beside_other_attributes() -> Result<(), String> {
    for (case, source) in [
        (
            "allow-then-path",
            "#[allow(dead_code)]\n#[path = \"sub/x.rs\"]\nmod x;\n",
        ),
        (
            "path-then-allow",
            "#[path = \"sub/x.rs\"]\n#[allow(dead_code)]\nmod x;\n",
        ),
    ] {
        let scratch = Scratch::new(case)?;
        scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
        scratch.write("src/lib.rs", source)?;
        scratch.write("src/sub/x.rs", "pub fn x() {}\n")?;

        let found = findings(&scratch.root, &scratch.files()?);
        assert!(found.is_empty(), "{case}: unexpected findings: {found:?}");
    }
    Ok(())
}

/// Every `#[cfg_attr]` branch of a `#[path]` pair is reachable, because the
/// branch that is compiled out on the current platform still counts.
#[test]
fn reaches_every_cfg_attr_path_branch() -> Result<(), String> {
    let scratch = Scratch::new("cfg-attr-pair")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write(
        "src/lib.rs",
        "#[cfg_attr(unix, path = \"a.rs\")]\n#[cfg_attr(not(unix), path = \"b.rs\")]\nmod pick;\n",
    )?;
    scratch.write("src/a.rs", "pub fn a() {}\n")?;
    scratch.write("src/b.rs", "pub fn b() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}

/// A `#[path]` value is authored relative to the file carrying it, so it may
/// escape that directory. The reported path must match the collected tree.
#[test]
fn resolves_a_parent_relative_path_attribute() -> Result<(), String> {
    let scratch = Scratch::new("parent-relative")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod nest;\n")?;
    scratch.write("src/nest/mod.rs", "#[path = \"../other/y.rs\"]\nmod y;\n")?;
    scratch.write("src/other/y.rs", "pub fn y() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}

/// Two counter-intuitive ground truths, pinned so a future "cleanup" cannot
/// reverse them: children of an inline `mod r#type { ... }` resolve in the
/// *unprefixed* `type/` directory, and only that spelling compiles.
#[test]
fn inline_raw_identifier_module_nests_in_the_unprefixed_directory() -> Result<(), String> {
    let scratch = Scratch::new("inline-raw-ident")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod r#type {\n    mod child;\n}\n")?;
    scratch.write("src/type/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/rtype/child.rs", "pub fn stranded() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(
        found.len(),
        1,
        "only the unprefixed `type/` directory is reachable: {found:?}"
    );
    assert_eq!(found[0].path, "src/rtype/child.rs");
    Ok(())
}

/// The repository's own controls: the real tree must be green, and the files
/// exercising each resolution rule must be present so it is not vacuous.
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

/// An inline module whose `#[path]` names a **directory** keeps its children
/// there and owns no file itself: `rustc` 1.97.1 compiles `src/thread/child.rs`,
/// so reporting it is a false positive. The decoy under the declaration's own
/// name proves the lookup uses the `#[path]` target, not a stem derived from `m`.
#[test]
fn inline_path_attribute_names_its_childrens_directory() -> Result<(), String> {
    let scratch = Scratch::new("inline-path-dir")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write(
        "src/lib.rs",
        "#[path = \"thread\"]\npub mod m {\n    pub mod child;\n}\n",
    )?;
    scratch.write("src/thread/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/m/child.rs", "pub fn decoy() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1, "expected only the decoy: {found:?}");
    assert_eq!(found[0].path, "src/m/child.rs");
    Ok(())
}

/// The same shape with the trailing-slash spelling and a nested `child/mod.rs`
/// child: `rustc` loads `src/thread/child/mod.rs`, so the ordinary
/// `NAME.rs`-then-`NAME/mod.rs` lookup must run *inside* the named directory.
#[test]
fn inline_path_attribute_keeps_the_ordinary_child_lookup() -> Result<(), String> {
    let scratch = Scratch::new("inline-path-nested")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write(
        "src/lib.rs",
        "#[path = \"thread/\"]\npub mod m {\n    pub mod child;\n}\n",
    )?;
    scratch.write("src/thread/child/mod.rs", "pub fn child() {}\n")?;
    scratch.write("src/orphan.rs", "pub fn orphan() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1, "expected only the orphan: {found:?}");
    assert_eq!(found[0].path, "src/orphan.rs");
    Ok(())
}

/// An inline `#[path]` inside another inline module resolves against its
/// enclosing directory: `rustc` compiles `src/x/a/thread/child.rs` for this
/// tree, not `src/thread/child.rs`. Both decoys are asserted, because a base
/// folded the wrong way still reports *a* finding — just the wrong one.
#[test]
fn nested_inline_path_attribute_keeps_its_enclosing_directories() -> Result<(), String> {
    let scratch = Scratch::new("nested-inline-path")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod x;\n")?;
    scratch.write(
        "src/x.rs",
        "mod a {\n    #[path = \"thread\"]\n    pub mod b {\n        pub mod child;\n    }\n}\n",
    )?;
    scratch.write("src/x/a/thread/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/thread/child.rs", "pub fn decoy() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1, "expected only the decoy: {found:?}");
    assert_eq!(found[0].path, "src/thread/child.rs");
    Ok(())
}

/// An inline `#[path]` in a *non-root* file resolves against that file's own
/// directory, not its `x/` module directory: `rustc` compiles `src/thread/child.rs`
/// and never `src/x/thread/child.rs`.
#[test]
fn unnested_inline_path_attribute_uses_the_carrying_files_directory() -> Result<(), String> {
    let scratch = Scratch::new("unnested-inline-path")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write("src/lib.rs", "mod x;\n")?;
    scratch.write(
        "src/x.rs",
        "#[path = \"thread\"]\npub mod m {\n    pub mod child;\n}\n",
    )?;
    scratch.write("src/thread/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/x/thread/child.rs", "pub fn decoy() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    assert_eq!(found.len(), 1, "expected only the decoy: {found:?}");
    assert_eq!(found[0].path, "src/x/thread/child.rs");
    Ok(())
}

/// An inline block owns no file, `#[path]` or not, so a file spelled where one
/// might be is dead text, and a `#[path]` written inside a block anchors at the
/// directory that block contributed. `rustc` 1.97.1 reads neither
/// `src/thread/other.rs`'s sibling `src/other.rs` nor `src/m.rs`,
/// `src/m/mod.rs`, or `src/m/m.rs` for `mod m { mod child; }` (all poisoned,
/// exit 0), nor any file at an inline `#[path = "t.rs"]`'s own value — the last
/// is `sts2-gateway#107`. Reporting a live file or crediting an orphan is the
/// delete-a-live-file direction.
#[test]
fn an_inline_block_owns_no_file_of_any_spelling() -> Result<(), String> {
    for (case, source, files, orphans) in [
        (
            "dir-path-then-file-path",
            "#[path = \"thread\"]\nmod m {\n    #[path = \"other.rs\"]\n    pub mod n;\n}\n",
            &[
                ("src/thread/other.rs", "pub fn live() {}\n"),
                ("src/other.rs", "not rust at all\n"),
            ][..],
            &["src/other.rs"][..],
        ),
        (
            "plain-block-then-file-path",
            "pub mod a {\n    #[path = \"x.rs\"]\n    pub mod m;\n}\n",
            &[
                ("src/a/x.rs", "pub fn live() {}\n"),
                ("src/x.rs", "not rust at all\n"),
            ][..],
            &["src/x.rs"][..],
        ),
        (
            "plain-inline",
            "mod m {\n    mod child;\n}\n",
            &[
                ("src/m/child.rs", "pub fn child() {}\n"),
                ("src/m.rs", "pub fn file() {}\n"),
                ("src/m/mod.rs", "pub fn directory() {}\n"),
                ("src/m/m.rs", "pub fn sibling() {}\n"),
            ][..],
            &["src/m.rs", "src/m/m.rs", "src/m/mod.rs"][..],
        ),
        (
            "path-at-value",
            "#[path = \"t.rs\"]\nmod m {}\n",
            &[("src/t.rs", "not rust at all\n")][..],
            &["src/t.rs"][..],
        ),
    ] {
        let scratch = Scratch::new(case)?;
        scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
        scratch.write("src/lib.rs", source)?;
        for (path, body) in files {
            scratch.write(path, body)?;
        }
        let found = findings(&scratch.root, &scratch.files()?);
        let mut paths: Vec<&str> = found.iter().map(|finding| finding.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(
            paths, orphans,
            "{case}: an inline block owns nothing: {found:?}"
        );
    }
    Ok(())
}

/// The rule must not be silenced by the fix: a genuine orphan is still reported,
/// and a **semicolon** `#[path]` targeting a directory must not be treated as
/// reaching `DIR/mod.rs`. `rustc` rejects that tree outright (`couldn't read
/// 'src/nest': Is a directory`), so nothing validates `src/nest/mod.rs`.
///
/// The inline block sits inside `src/lib.rs`'s own `m2`, so rustc compiles
/// `src/m2/thread/child.rs` — marker-verified: an error there fails the build,
/// the same error in `src/thread/child.rs` is never read. This test predates
/// that check and reported a file rustc does not compile, exactly the
/// false-positive class `#105` is about; the fixture is corrected, not the
/// assertion.
#[test]
fn inline_path_support_keeps_true_orphans_and_the_semicolon_arm() -> Result<(), String> {
    let scratch = Scratch::new("inline-path-orphan")?;
    scratch.write("Cargo.toml", "[package]\nname = \"scratch\"\n")?;
    scratch.write(
        "src/lib.rs",
        "mod semi;\npub mod m2 {\n    #[path = \"thread\"]\n    pub mod m {\n        pub mod child;\n    }\n}\n",
    )?;
    scratch.write("src/m2/thread/child.rs", "pub fn child() {}\n")?;
    scratch.write("src/orphan.rs", "pub fn orphan() {}\n")?;
    scratch.write("src/semi.rs", "#[path = \"nest\"]\nmod inner;\n")?;
    scratch.write("src/nest/mod.rs", "pub fn inner() {}\n")?;

    let found = findings(&scratch.root, &scratch.files()?);
    let mut paths: Vec<&str> = found.iter().map(|finding| finding.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec!["src/nest/mod.rs", "src/orphan.rs"],
        "expected the two genuine orphans: {found:?}"
    );
    Ok(())
}
