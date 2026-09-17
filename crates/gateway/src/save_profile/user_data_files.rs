// SPDX-License-Identifier: MIT

use std::fs;
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
        fs::create_dir(&directory).map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => UserDataPortError::ExistingContents,
            _ => UserDataPortError::Unavailable,
        })?;
        let record = serde_json::json!({
            "identity": identity.value(),
            "provenance": provenance,
        });
        let bytes = serde_json::to_vec(&record).map_err(|_| UserDataPortError::UnknownContents)?;
        fs::write(directory.join(PROVENANCE_FILE), bytes)
            .map_err(|_| UserDataPortError::Unavailable)?;
        Ok(())
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
        inspect_identity(&self.root, identity)
    }

    fn create(
        &mut self,
        request: UserDataCreateRequest,
    ) -> Result<UserDataCreateOutcome, UserDataPortError> {
        let descriptor = request.descriptor;
        let identity = descriptor.identity;
        if descriptor.validate().is_err()
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
