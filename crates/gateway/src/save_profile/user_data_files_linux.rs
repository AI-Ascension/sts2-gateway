// SPDX-License-Identifier: MIT

//! Linux-only handle-pinned partial provisioning hardening; no unsafe FFI.
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use super::super::provisioning_types::{UserDataInspection, UserDataPortError};
use super::super::types::{UserDataIdentity, UserDataProvenance};
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

/// Open every approved-root ancestor relative to a pinned directory handle.
///
/// The ordinary path check in `verify_root` is not a race-free open. Here each
/// next component is a single O_NOFOLLOW/O_DIRECTORY lookup beneath the prior
/// pinned descriptor, so an intervening ancestor swap cannot redirect the
/// actual approved handle. This is not hostile same-UID process isolation.
#[cfg(target_os = "linux")]
pub(super) fn pinned_approved_root(path: &Path) -> Result<File, UserDataPortError> {
    use std::os::fd::AsRawFd;
    use std::path::Component;

    if !path.is_absolute() {
        return Err(UserDataPortError::SymlinkEscape);
    }
    let mut directory = pinned_directory(Path::new("/"))?;
    for component in path.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(UserDataPortError::SymlinkEscape),
        };
        let child = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name);
        directory = pinned_directory(&child)?;
    }
    if !same_directory_identity(&directory, path) {
        return Err(UserDataPortError::SymlinkEscape);
    }
    Ok(directory)
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

/// The newly-created marker must still be the original inode after readback.
/// A replacement file with identical valid owner JSON is not this creation.
fn same_regular_file_identity(created: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(original), Ok(named)) = (created.metadata(), fs::symlink_metadata(path)) else {
        return false;
    };
    original.is_file()
        && named.is_file()
        && !named.file_type().is_symlink()
        && original.nlink() == 1
        && named.nlink() == 1
        && original.dev() == named.dev()
        && original.ino() == named.ino()
}

impl FilesystemUserDataPort {
    /// Recheck the original created inode after replay/sync, not a same-name replacement.
    /// This is fail-closed race detection, not isolation against a hostile same-UID peer.
    pub(super) fn verify_created_child_after_sync(
        &self,
        created: &File,
        created_marker: &File,
        path: &Path,
        identity: UserDataIdentity,
        expected: &UserDataProvenance,
    ) -> Result<(), UserDataPortError> {
        use std::os::fd::AsRawFd;
        if !same_directory_identity(created, path)
            || !same_directory_identity(&self.root_handle, &self.root)
        {
            return Err(UserDataPortError::TimeoutAfterWrite);
        }
        let pinned = PathBuf::from(format!("/proc/self/fd/{}", created.as_raw_fd()));
        let marker = pinned.join(super::PROVENANCE_FILE);
        if !same_regular_file_identity(created_marker, &marker)
            || read_owned(&pinned, identity)
                .map_err(|_| UserDataPortError::TimeoutAfterWrite)?
                .as_ref()
                != Some(expected)
            || !same_regular_file_identity(created_marker, &marker)
        {
            return Err(UserDataPortError::TimeoutAfterWrite);
        }
        Ok(())
    }

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
#[path = "user_data_files_linux_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "user_data_files_root_ancestor_tests.rs"]
mod root_ancestor_tests;
