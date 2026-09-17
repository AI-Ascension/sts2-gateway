// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use super::provisioning_types::{UserDataInspection, UserDataPortError};
use super::types::{UserDataIdentity, UserDataProvenance};

/// Directory-relative name of the gateway-owned provenance record.
pub(super) const PROVENANCE_FILE: &str = "gateway-provenance.json";

/// Resolves and verifies the server-owned root once per port construction.
///
/// The returned path is canonical, so later containment checks compare canonical forms and a
/// root whose own path traverses a symlink is refused instead of silently followed.
pub(super) fn verify_root(root: &Path) -> Result<PathBuf, UserDataPortError> {
    if !root.is_absolute() {
        return Err(UserDataPortError::Traversal);
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| UserDataPortError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(UserDataPortError::SymlinkEscape);
    }
    let canonical = fs::canonicalize(root).map_err(|_| UserDataPortError::Unavailable)?;
    if canonical != root {
        return Err(UserDataPortError::SymlinkEscape);
    }
    Ok(canonical)
}

/// Rejects identity values that cannot name a server-owned allocation.
pub(super) fn verify_identity(identity: UserDataIdentity) -> Result<(), UserDataPortError> {
    if identity.value() == 0 {
        return Err(UserDataPortError::Traversal);
    }
    Ok(())
}

/// The only allocation directory an identity may ever name inside the canonical root.
pub(super) fn allocation_directory(root: &Path, identity: UserDataIdentity) -> PathBuf {
    root.join(format!("run-{}", identity.value()))
}

/// Refuses a child that is not a real directory directly under the canonical root.
///
/// Symlinks are rejected before canonicalization, so a symlink swapped in below the root is
/// refused as an escape rather than followed. A missing child is rejected when `existing`.
pub(super) fn verify_containment(
    root: &Path,
    child: &Path,
    existing: bool,
) -> Result<(), UserDataPortError> {
    if !child.is_absolute()
        || !root.is_absolute()
        || child.parent() != Some(root)
        || root.canonicalize().ok().as_deref() != Some(root)
    {
        return Err(UserDataPortError::Traversal);
    }
    match fs::symlink_metadata(child) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(UserDataPortError::SymlinkEscape);
            }
            if !metadata.is_dir() {
                return Err(UserDataPortError::UnknownContents);
            }
            let canonical_child =
                fs::canonicalize(child).map_err(|_| UserDataPortError::Unavailable)?;
            if canonical_child.parent() != Some(root) {
                return Err(UserDataPortError::SymlinkEscape);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if existing {
                return Err(UserDataPortError::Unavailable);
            }
        }
        Err(_) => return Err(UserDataPortError::Unavailable),
    }
    Ok(())
}

/// Reads and validates the provenance file of an existing allocation directory.
pub(super) fn read_owned(
    directory: &Path,
    identity: UserDataIdentity,
) -> Result<Option<UserDataProvenance>, UserDataPortError> {
    let bytes = match fs::read(directory.join(PROVENANCE_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UserDataPortError::Unavailable),
    };
    let recorded: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| UserDataPortError::UnknownContents)?;
    let Some(object) = recorded.as_object() else {
        return Err(UserDataPortError::UnknownContents);
    };
    if object.len() != 2
        || object.get("identity").and_then(serde_json::Value::as_u64) != Some(identity.value())
    {
        return Err(UserDataPortError::UnknownContents);
    }
    let provenance: UserDataProvenance = serde_json::from_value(
        object
            .get("provenance")
            .cloned()
            .ok_or(UserDataPortError::UnknownContents)?,
    )
    .map_err(|_| UserDataPortError::UnknownContents)?;
    provenance
        .validate()
        .map_err(|_| UserDataPortError::UnknownContents)?;
    Ok(Some(provenance))
}

/// True when the allocation directory does not exist yet.
pub(super) fn is_absent(directory: &Path) -> bool {
    matches!(
        fs::symlink_metadata(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Inspects one identity without writing, mapping the observation to a typed classification.
pub(super) fn inspect_identity(
    root: &Path,
    identity: UserDataIdentity,
) -> Result<UserDataInspection, UserDataPortError> {
    verify_identity(identity)?;
    let child = allocation_directory(root, identity);
    match verify_containment(root, &child, false) {
        Ok(()) => {}
        Err(UserDataPortError::Unavailable) => return Ok(UserDataInspection::Absent),
        Err(error) => return Err(error),
    }
    if is_absent(&child) {
        return Ok(UserDataInspection::Absent);
    }
    match read_owned(&child, identity)? {
        Some(provenance) => Ok(UserDataInspection::Owned(provenance)),
        None => Ok(UserDataInspection::UnknownContents),
    }
}
