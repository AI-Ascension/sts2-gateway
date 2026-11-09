// SPDX-License-Identifier: MIT

//! Linux-only handle-pinned partial provisioning hardening; no unsafe FFI.
use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use super::super::provisioning_types::{UserDataInspection, UserDataPortError};
use super::super::types::UserDataIdentity;
use super::super::user_data_provenance::read_owned;
use super::FilesystemUserDataPort;

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

impl FilesystemUserDataPort {
    pub(super) fn inspect_pinned(
        &self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError> {
        use std::os::fd::AsRawFd;

        // Read only through the originally approved root descriptor. Replacing a
        // pathname must never redirect provenance reads to an unrelated root.
        self.ensure_original_root()?;
        let child_path = PathBuf::from(format!(
            "/proc/self/fd/{}/run-{}",
            self.root_handle.as_raw_fd(),
            identity.value()
        ));
        match fs::symlink_metadata(&child_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.ensure_original_root()?;
                return Ok(UserDataInspection::Absent);
            }
            Err(_) => return Err(UserDataPortError::Unavailable),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(UserDataPortError::SymlinkEscape);
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(UserDataPortError::UnknownContents);
            }
            Ok(_) => {}
        }

        let child = pinned_directory(&child_path)?;
        let pinned_child = PathBuf::from(format!("/proc/self/fd/{}", child.as_raw_fd()));
        let provenance = read_owned(&pinned_child, identity)?;
        if !same_directory_identity(&child, &child_path) {
            return Err(UserDataPortError::SymlinkEscape);
        }
        self.ensure_original_root()?;
        Ok(match provenance {
            Some(value) => UserDataInspection::Owned(value),
            None => UserDataInspection::UnknownContents,
        })
    }
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
    fn replacing_approved_root_with_regular_directory_is_refused() -> std::io::Result<()> {
        use super::super::super::provisioning_types::{UserDataPort, UserDataPortError};
        use super::super::super::types::UserDataIdentity;
        use super::super::FilesystemUserDataPort;

        let base =
            std::env::temp_dir().join(format!("sts2-pinned-replaced-root-{}", Uuid::new_v4()));
        let root = base.join("root");
        fs::create_dir_all(&root)?;
        let mut port = FilesystemUserDataPort::try_new(&root, 8)
            .map_err(|_| std::io::Error::other("could not bind initial root"))?;
        fs::rename(&root, base.join("former-root"))?;
        fs::create_dir(&root)?;
        assert_eq!(
            port.inspect(UserDataIdentity::new(1)),
            Err(UserDataPortError::SymlinkEscape),
            "a new real directory at an approved pathname is not the originally approved inode"
        );
        assert!(fs::read_dir(&root)?.next().is_none());
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
