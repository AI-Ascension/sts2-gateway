// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::provisioning_types::{
    UserDataCreateOutcome, UserDataCreateRequest, UserDataInspection, UserDataPort,
    UserDataPortError,
};
use super::types::{UserDataIdentity, UserDataProvenance};
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
}

impl FilesystemUserDataPort {
    /// Binds the port to an absolute, canonical, server-owned root path.
    pub fn try_new(root: impl Into<PathBuf>, capacity: usize) -> Result<Self, UserDataPortError> {
        if capacity == 0 || capacity > USER_DATA_ROOT_CAPACITY {
            return Err(UserDataPortError::Unavailable);
        }
        Ok(Self {
            root: verify_root(&root.into())?,
            capacity,
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
        let directory = self.directory(identity);
        self.verify(identity, false)?;
        let bytes = super::user_data_provenance::encode(identity, provenance)?;
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
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // Every error from this point follows a physical effect, so retain an unknown outcome.
        let mut file = options
            .open(directory.join(PROVENANCE_FILE))
            .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| File::open(&directory)?.sync_all())
            .and_then(|()| File::open(&self.root)?.sync_all())
            .map_err(|_| UserDataPortError::TimeoutAfterWrite)?;
        Ok(())
    }

    fn sync_owned(&self, identity: UserDataIdentity) -> Result<(), UserDataPortError> {
        let directory = self.directory(identity);
        File::open(directory.join(PROVENANCE_FILE))
            .and_then(|file| file.sync_all())
            .and_then(|()| File::open(&directory)?.sync_all())
            .and_then(|()| File::open(&self.root)?.sync_all())
            .map_err(|_| UserDataPortError::TimeoutAfterWrite)
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
