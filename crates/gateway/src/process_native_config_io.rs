// SPDX-License-Identifier: MIT

use super::NativeProcessError;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

pub(super) fn protected_path(path: &Path) -> Result<std::path::PathBuf, NativeProcessError> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(NativeProcessError::Configuration);
    }
    match fs::canonicalize(path) {
        Ok(resolved) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(NativeProcessError::Configuration)?;
            let name = path.file_name().ok_or(NativeProcessError::Configuration)?;
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            Ok(fs::canonicalize(parent)
                .map_err(|_| NativeProcessError::Configuration)?
                .join(name))
        }
        Err(_) => Err(NativeProcessError::Configuration),
    }
}

const MAX_IMAGE_BYTES: u64 = 536_870_912;

pub(in crate::process_native) fn exact_path(
    path: &Path,
    directory: bool,
) -> Result<(), NativeProcessError> {
    if !path.is_absolute()
        || path.to_str().is_none_or(|text| {
            text.contains(['\0', '%']) || text.bytes().any(|byte| byte.is_ascii_whitespace())
        })
    {
        return Err(NativeProcessError::Configuration);
    }
    if fs::canonicalize(path).map_err(|_| NativeProcessError::Configuration)? != path {
        return Err(NativeProcessError::Configuration);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| NativeProcessError::Configuration)?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(NativeProcessError::Configuration);
    }
    Ok(())
}

pub(in crate::process_native) fn private_directory(path: &Path) -> Result<(), NativeProcessError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)
            .map_err(|_| NativeProcessError::Configuration)?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(NativeProcessError::Configuration);
        }
    }
    Ok(())
}

pub(super) fn executable_file(path: &Path) -> Result<(), NativeProcessError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)
            .map_err(|_| NativeProcessError::Configuration)?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(NativeProcessError::Profile);
        }
    }
    Ok(())
}

pub(in crate::process_native) fn bounded_read(
    path: &Path,
    limit: u64,
) -> Result<Vec<u8>, NativeProcessError> {
    let file = File::open(path).map_err(|_| NativeProcessError::Configuration)?;
    if !file
        .metadata()
        .map_err(|_| NativeProcessError::Configuration)?
        .is_file()
    {
        return Err(NativeProcessError::Configuration);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| NativeProcessError::Configuration)?;
    if bytes.len() as u64 > limit {
        return Err(NativeProcessError::Configuration);
    }
    Ok(bytes)
}

pub(in crate::process_native) fn image_digest(path: &Path) -> Result<String, NativeProcessError> {
    let mut file = File::open(path).map_err(|_| NativeProcessError::Profile)?;
    let metadata = file.metadata().map_err(|_| NativeProcessError::Profile)?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err(NativeProcessError::Profile);
    }
    let mut digest = Sha256::new();
    let mut bytes = [0; 65_536];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut bytes)
            .map_err(|_| NativeProcessError::Profile)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_IMAGE_BYTES {
            return Err(NativeProcessError::Profile);
        }
        digest.update(&bytes[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(in crate::process_native) fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub(in crate::process_native) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
