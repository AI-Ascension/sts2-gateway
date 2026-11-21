// SPDX-License-Identifier: MIT

//! Linux approved-root component pinning; no native STS2 owner is exercised.

use super::{pinned_approved_root, same_directory_identity};
use std::fs;
use std::os::unix::fs::symlink;
use uuid::Uuid;

#[test]
fn an_intermediate_symlink_is_not_an_approved_root() -> std::io::Result<()> {
    let base = std::env::temp_dir().join(format!("sts2-root-chain-{}", Uuid::new_v4()));
    let root = base.join("real/owned");
    fs::create_dir_all(&root)?;
    symlink(base.join("real"), base.join("alias"))?;
    assert!(pinned_approved_root(&root).is_ok());
    assert!(
        pinned_approved_root(&base.join("alias/owned")).is_err(),
        "an intermediate symlink must not become a root capability"
    );
    fs::remove_dir_all(base)
}

#[test]
fn renamed_ancestor_does_not_rebind_original_root_handle() -> std::io::Result<()> {
    let base = std::env::temp_dir().join(format!("sts2-root-ancestor-{}", Uuid::new_v4()));
    let parent = base.join("parent");
    let root = parent.join("owned");
    fs::create_dir_all(&root)?;
    let original = pinned_approved_root(&root)
        .map_err(|_| std::io::Error::other("could not pin original root"))?;
    assert!(same_directory_identity(&original, &root));

    fs::rename(&parent, base.join("moved"))?;
    fs::create_dir(&parent)?;
    symlink(base.join("moved/owned"), &root)?;
    assert!(
        !same_directory_identity(&original, &root),
        "a replaced ancestor cannot attest the original root"
    );
    assert!(pinned_approved_root(&root).is_err());

    // The original rooted inode still exists; neither test performs an owner
    // allocation, launches a game, or treats absence as an idle witness.
    assert!(base.join("moved/owned").is_dir());
    fs::remove_dir_all(base)
}
