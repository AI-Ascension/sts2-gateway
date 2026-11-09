// SPDX-License-Identifier: MIT

//! Linux-only handle-pinned partial provisioning hardening; no unsafe FFI.
use std::fs::{self, File, OpenOptions};
use std::path::Path;

use super::provisioning_types::UserDataPortError;

#[cfg(target_os = "linux")]
pub(super) fn pinned_directory(path: &Path) -> Result<File, UserDataPortError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let before = fs::symlink_metadata(path).map_err(|_| UserDataPortError::Unavailable)?;
    if before.file_type().is_symlink() || !before.is_dir() {
        return Err(UserDataPortError::SymlinkEscape);
    }
    // Linux O_DIRECTORY | O_NOFOLLOW. No unsafe FFI or extra dependency.
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(0o600000);
    let opened = options
        .open(path)
        .map_err(|_| UserDataPortError::SymlinkEscape)?;
    let actual = opened
        .metadata()
        .map_err(|_| UserDataPortError::Unavailable)?;
    if !actual.is_dir() || before.dev() != actual.dev() || before.ino() != actual.ino() {
        return Err(UserDataPortError::SymlinkEscape);
    }
    Ok(opened)
}

#[cfg(target_os = "linux")]
pub(super) fn same_directory_identity(opened: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(actual), Ok(named)) = (opened.metadata(), fs::symlink_metadata(path)) else {
        return false;
    };
    named.is_dir()
        && !named.file_type().is_symlink()
        && actual.dev() == named.dev()
        && actual.ino() == named.ino()
}

#[cfg(test)]
mod tests {
    use super::super::PROVENANCE_FILE;
    use super::pinned_directory;
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn swapped_child_symlink_is_not_followed() -> std::io::Result<()> {
        let base = std::env::temp_dir().join(format!("sts2-pinned-child-{}", Uuid::new_v4()));
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root)?;
        fs::create_dir(&outside)?;
        let parent = pinned_directory(&root)
            .map_err(|_| std::io::Error::other("could not pin fixture root"))?;
        let child = PathBuf::from(format!("/proc/self/fd/{}/run-1", parent.as_raw_fd()));
        fs::create_dir(&child)?;
        fs::rename(&child, root.join("moved"))?;
        symlink(&outside, &child)?;
        assert!(pinned_directory(&child).is_err());
        assert!(!outside.join(PROVENANCE_FILE).exists());
        fs::remove_dir_all(&base)
    }

    #[test]
    fn renamed_root_cannot_redirect_pinned_creation() -> std::io::Result<()> {
        let base = std::env::temp_dir().join(format!("sts2-pinned-root-{}", Uuid::new_v4()));
        let root = base.join("root");
        let moved = base.join("moved");
        let outside = base.join("outside");
        fs::create_dir_all(&root)?;
        fs::create_dir(&outside)?;
        let parent = pinned_directory(&root)
            .map_err(|_| std::io::Error::other("could not pin fixture root"))?;
        fs::rename(&root, &moved)?;
        symlink(&outside, &root)?;
        fs::create_dir(PathBuf::from(format!(
            "/proc/self/fd/{}/run-1",
            parent.as_raw_fd()
        )))?;
        assert!(moved.join("run-1").is_dir());
        assert!(!outside.join("run-1").exists());
        fs::remove_dir_all(&base)
    }
}
