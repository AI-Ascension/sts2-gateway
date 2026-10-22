// SPDX-License-Identifier: MIT

use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use super::provisioning_types::UserDataPortError;
use super::types::{LAUNCH_PROFILE_CONTRACT, UserDataIdentity, UserDataProvenance};

pub(super) const PROVENANCE_FILE: &str = "gateway-provenance.json";
const MAX_PROVENANCE_BYTES: u64 = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    identity: u64,
    provenance: UserDataProvenance,
}

pub(super) fn validate(provenance: &UserDataProvenance) -> Result<(), UserDataPortError> {
    provenance
        .validate()
        .map_err(|_| UserDataPortError::UnknownContents)?;
    if provenance.owner != super::provisioning_descriptor::PROVENANCE_OWNER
        || provenance.contract != LAUNCH_PROFILE_CONTRACT
    {
        return Err(UserDataPortError::UnknownContents);
    }
    Ok(())
}

pub(super) fn encode(
    identity: UserDataIdentity,
    provenance: &UserDataProvenance,
) -> Result<Vec<u8>, UserDataPortError> {
    validate(provenance)?;
    // Preserve the previous writer's sorted-object spelling.
    serde_json::to_vec(&serde_json::json!({
        "identity": identity.value(),
        "provenance": provenance,
    }))
    .map_err(|_| UserDataPortError::UnknownContents)
}

/// The deployment exclusively owns the root; before/after metadata also refuses static links
/// and a changed file identity before any bytes can be accepted as an ownership record.
pub(super) fn read_owned(
    directory: &Path,
    identity: UserDataIdentity,
) -> Result<Option<UserDataProvenance>, UserDataPortError> {
    let path = directory.join(PROVENANCE_FILE);
    let before = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(UserDataPortError::Unavailable),
    };
    regular_record(&before)?;
    let file = File::open(&path).map_err(|_| UserDataPortError::Unavailable)?;
    let opened = file
        .metadata()
        .map_err(|_| UserDataPortError::Unavailable)?;
    regular_record(&opened)?;
    if !same_file(&before, &opened) {
        return Err(UserDataPortError::UnknownContents);
    }
    let mut bytes = Vec::new();
    file.take(MAX_PROVENANCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| UserDataPortError::Unavailable)?;
    let after = fs::symlink_metadata(path).map_err(|_| UserDataPortError::Unavailable)?;
    regular_record(&after)?;
    if bytes.len() as u64 > MAX_PROVENANCE_BYTES || !same_file(&opened, &after) {
        return Err(UserDataPortError::UnknownContents);
    }
    let record: Record =
        serde_json::from_slice(&bytes).map_err(|_| UserDataPortError::UnknownContents)?;
    if record.identity != identity.value() {
        return Err(UserDataPortError::UnknownContents);
    }
    validate(&record.provenance)?;
    Ok(Some(record.provenance))
}

fn regular_record(metadata: &Metadata) -> Result<(), UserDataPortError> {
    if metadata.file_type().is_symlink() {
        return Err(UserDataPortError::SymlinkEscape);
    }
    if !metadata.is_file() || metadata.len() > MAX_PROVENANCE_BYTES {
        return Err(UserDataPortError::UnknownContents);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(UserDataPortError::UnknownContents);
        }
    }
    Ok(())
}

fn same_file(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev() || left.ino() != right.ino() {
            return false;
        }
    }
    left.len() == right.len()
        && left
            .modified()
            .ok()
            .is_some_and(|modified| right.modified().ok() == Some(modified))
}
