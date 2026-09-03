// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sts2_gateway::RuntimeV2PersistedState;

const JOURNAL_FORMAT_VERSION: u32 = 1;
const MAX_JOURNAL_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct JournalLock {
    file: File,
}

impl JournalLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self, String> {
        ensure_parent_directory(path)?;
        let lock_path = path.with_extension("runtime-v2.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| format!("Runtime-v2 journal lock open failed: {error}"))?;
        file.try_lock().map_err(|error| {
            format!("Runtime-v2 journal is already locked or unavailable: {error}")
        })?;
        Ok(Self { file })
    }
}

impl Drop for JournalLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalDocument {
    format_version: u32,
    state: RuntimeV2PersistedState,
}

pub(crate) fn load(path: &Path) -> Result<Option<RuntimeV2PersistedState>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Runtime-v2 journal read failed: {error}")),
    };
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(String::from("Runtime-v2 journal exceeds its size bound"));
    }
    let document = serde_json::from_slice::<JournalDocument>(&bytes)
        .map_err(|error| format!("Runtime-v2 journal is malformed: {error}"))?;
    if document.format_version != JOURNAL_FORMAT_VERSION {
        return Err(format!(
            "unsupported Runtime-v2 journal format {}",
            document.format_version
        ));
    }
    Ok(Some(document.state))
}

pub(crate) fn store(path: &Path, state: &RuntimeV2PersistedState) -> Result<(), String> {
    let document = JournalDocument {
        format_version: JOURNAL_FORMAT_VERSION,
        state: state.clone(),
    };
    let bytes = serde_json::to_vec(&document)
        .map_err(|error| format!("Runtime-v2 journal serialization failed: {error}"))?;
    if bytes.len() > MAX_JOURNAL_BYTES {
        return Err(String::from("Runtime-v2 journal exceeds its size bound"));
    }
    ensure_parent_directory(path)?;
    let temporary_path = path.with_extension("runtime-v2.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)
        .map_err(|error| format!("Runtime-v2 journal temporary file open failed: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("Runtime-v2 journal write failed: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Runtime-v2 journal sync failed: {error}"))?;
    drop(file);
    fs::rename(&temporary_path, path)
        .map_err(|error| format!("Runtime-v2 journal replace failed: {error}"))?;
    sync_parent_directory(path)
}

fn ensure_parent_directory(path: &Path) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Runtime-v2 journal directory creation failed: {error}"))?;
    }
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<(), String> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Runtime-v2 journal directory sync failed: {error}"))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sts2_gateway::{RuntimeV2CombatPhase, RuntimeV2Observation};

    use super::{JournalLock, load, store};

    fn test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "sts2-runtime-v2-journal-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn journal_round_trips_bounded_state() -> Result<(), String> {
        let path = test_path();
        let state = sts2_gateway::RuntimeV2PersistedState {
            instance_id: String::from("instance-1"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            observation: RuntimeV2Observation::new(
                RuntimeV2CombatPhase::OutsideCombat,
                0,
                false,
                0,
            ),
            operations: Vec::new(),
        };
        let result = (|| {
            store(&path, &state)?;
            let loaded = load(&path)?.ok_or_else(|| String::from("journal was not loaded"))?;
            if loaded != state {
                return Err(String::from("journal state did not round-trip"));
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("runtime-v2.tmp"));
        result
    }

    #[test]
    fn journal_lock_is_exclusive_until_released() -> Result<(), String> {
        let path = test_path();
        let result = (|| {
            let lock = JournalLock::acquire(&path)?;
            if JournalLock::acquire(&path).is_ok() {
                return Err(String::from("journal lock was not exclusive"));
            }
            drop(lock);
            let reacquired = JournalLock::acquire(&path)?;
            drop(reacquired);
            Ok(())
        })();
        let _ = std::fs::remove_file(path.with_extension("runtime-v2.lock"));
        result
    }
}
