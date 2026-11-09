// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::provisioning_types::{
    UserDataCreateOutcome, UserDataCreateRequest, UserDataInspection, UserDataPort,
    UserDataPortError,
};
use super::types::{UserDataIdentity, UserDataProvenance};
#[cfg(target_os = "linux")]
#[path = "user_data_files_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
use linux::{pinned_directory, same_directory_identity};

use super::user_data_files_validation::{
    PROVENANCE_FILE, allocation_directory, inspect_identity, is_absent, read_owned,
    verify_containment, verify_identity, verify_root,
};

/// Default bound on identities a single configured root will serve.
pub const USER_DATA_ROOT_CAPACITY: usize = 64;

/// Filesystem adapter that owns one bounded, server-configured isolated root.
///
/// The port allocates a fresh opaque directory per identity, records provenance inside that
/// directory, and refuses traversal, symlink escape, unknown existing contents, and implicit
/// overwrite or adoption. Only the opaque identity leaves the port; host paths never appear in a
/// descriptor or error.
/// The deployment must exclusively own the configured root and its ancestors. Creation is
/// acknowledged only after the provenance file and both directory entries have been synced.
pub struct FilesystemUserDataPort {
    root: PathBuf,
    capacity: usize,
    #[cfg(target_os = "linux")]
    root_handle: File,
}

impl FilesystemUserDataPort {
    /// Binds the port to an absolute, canonical, server-owned root path.
    pub fn try_new(root: impl Into<PathBuf>, capacity: usize) -> Result<Self, UserDataPortError> {
        if capacity == 0 || capacity > USER_DATA_ROOT_CAPACITY {
            return Err(UserDataPortError::Unavailable);
        }
        let root = verify_root(&root.into())?;
        #[cfg(target_os = "linux")]
        let root_handle = pinned_directory(&root)?;
        Ok(Self {
            root,
            capacity,
            #[cfg(target_os = "linux")]
            root_handle,
        })
    }

    /// Root path retained by the port; it never appears in a descriptor or wire response.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn directory(&self, identity: UserDataIdentity) -> PathBuf {
        allocation_directory(&self.root, identity)
    }

    fn verify(&self, identity: UserDataIdentity, existing: bool) -> Result<(), UserDataPortError> {
        #[cfg(target_os = "linux")]
        if !same_directory_identity(&self.root_handle, &self.root) {
            return Err(UserDataPortError::SymlinkEscape);
        }
        verify_identity(identity)?;
        if identity.value() > self.capacity as u64 {
            return Err(UserDataPortError::Traversal);
        }
        verify_containment(&self.root, &self.directory(identity), existing)
    }

    fn create_new(
        &self,
        identity: UserDataIdentity,
        provenance: &UserDataProvenance,
    ) -> Result<(), UserDataPortError> {
        self.verify(identity, false)?;
        let bytes = super::user_data_provenance::encode(identity, provenance)?;

        // On Linux, anchor both directory creation and marker I/O to open directory
        // descriptors. A replaced child pathname cannot redirect a marker write.
        // /proc/self/fd must be available; otherwise fail closed.
        #[cfg(target_os = "linux")]
        let parent = self
            .root_handle
            .try_clone()
            .map_err(|_| UserDataPortError::Unavailable)?;
        #[cfg(target_os = "linux")]
        let directory = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!(
                "/proc/self/fd/{}/run-{}",
                parent.as_raw_fd(),
                identity.value()
            ))
        };
        #[cfg(not(target_os = "linux"))]
        let directory = self.directory(identity);

        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => UserDataPortError::ExistingContents,
                _ => UserDataPortError::Unavailable,
            })?;

        // Any error below follows a physical creation. Never report settled.
        #[cfg(target_os = "linux")]
        let child =
            pinned_directory(&directory).map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
        #[cfg(target_os = "linux")]
        let marker = {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!(
                "/proc/self/fd/{}/{}",
                child.as_raw_fd(),
                PROVENANCE_FILE
            ))
        };
        #[cfg(not(target_os = "linux"))]
        let marker = directory.join(PROVENANCE_FILE);

        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0o400000); // O_NOFOLLOW on the marker itself
        }
        let mut file = options
            .open(&marker)
            .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;

        #[cfg(target_os = "linux")]
        {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .and_then(|()| child.sync_all())
                .and_then(|()| parent.sync_all())
                .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
            if !same_directory_identity(&child, &directory)
                || !same_directory_identity(&parent, &self.root)
            {
                return Err(UserDataPortError::TimeoutAfterWrite);
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .and_then(|()| File::open(&directory)?.sync_all())
                .and_then(|()| File::open(&self.root)?.sync_all())
                .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
        }
        Ok(())
    }

    fn sync_owned(&self, identity: UserDataIdentity) -> Result<(), UserDataPortError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let parent = self
            .root_handle
            .try_clone()
            .map_err(|_| UserDataPortError::Unavailable)?;
            let directory = PathBuf::from(format!(
                "/proc/self/fd/{}/run-{}",
                parent.as_raw_fd(),
                identity.value()
            ));
            let child = pinned_directory(&directory)?;
            let marker = PathBuf::from(format!(
                "/proc/self/fd/{}/{}",
                child.as_raw_fd(),
                PROVENANCE_FILE
            ));
            File::open(marker)
                .and_then(|file| file.sync_all())
                .and_then(|()| child.sync_all())
                .and_then(|()| parent.sync_all())
                .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
            if !same_directory_identity(&child, &directory)
                || !same_directory_identity(&parent, &self.root)
            {
                return Err(UserDataPortError::TimeoutAfterWrite);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let directory = self.directory(identity);
            File::open(directory.join(PROVENANCE_FILE))
                .and_then(|file| file.sync_all())
                .and_then(|()| File::open(&directory)?.sync_all())
                .and_then(|()| File::open(&self.root)?.sync_all())
                .map_err(|_| UserDataPortError::TimeoutAfterWrite)
        }
    }
}

impl UserDataPort for FilesystemUserDataPort {
    fn inspect(
        &mut self,
        identity: UserDataIdentity,
    ) -> Result<UserDataInspection, UserDataPortError> {
        verify_identity(identity)?;
        if identity.value() > self.capacity as u64 {
            return Err(UserDataPortError::Traversal);
        }
        #[cfg(target_os = "linux")]
        if !same_directory_identity(&self.root_handle, &self.root) {
            return Err(UserDataPortError::SymlinkEscape);
        }
        let inspected = inspect_identity(&self.root, identity)?;
        if matches!(inspected, UserDataInspection::Owned(_)) {
            self.sync_owned(identity)?;
        }
        Ok(inspected)
    }

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError> {
        let descriptor = request.descriptor;
        let identity = descriptor.identity;
        if descriptor.validate().is_err()
            || super::user_data_provenance::validate(&descriptor.provenance).is_err()
            || request.launch_profile.validate().is_err()
            || request.launch_profile.user_data != identity
        {
            return Err(UserDataPortError::UnknownContents);
        }
        self.verify(identity, false)?;
        let directory = self.directory(identity);
        match read_owned(&directory, identity)? {
            Some(provenance) => {
                return if provenance == descriptor.provenance {
                    self.sync_owned(identity)?;
                    Ok(UserDataCreateOutcome::AlreadyCreated(descriptor))
                } else {
                    Err(UserDataPortError::ExistingContents)
                };
            }
            None if !is_absent(&directory) => {
                return Err(UserDataPortError::ExistingContents);
            }
            None => self.create_new(identity, &descriptor.provenance)?,
        }
        Ok(UserDataCreateOutcome::Created(descriptor))
    }
}
