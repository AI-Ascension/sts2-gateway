// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io;

use fs2::FileExt;

/// Exclusive coordinator authority, released explicitly before its descriptor closes.
///
/// A forked helper can temporarily retain the same open-file description before exec closes
/// inherited descriptors. Closing only this descriptor would then prolong the owner's lock.
/// Store this guard after the SQLite connection so field teardown closes the database first.
#[derive(Debug)]
pub(crate) struct CoordinatorLock {
    file: File,
}

impl CoordinatorLock {
    pub(crate) fn acquire(file: File) -> io::Result<Self> {
        FileExt::try_lock_exclusive(&file)?;
        Ok(Self { file })
    }
}

impl Drop for CoordinatorLock {
    fn drop(&mut self) {
        // Failure retains the existing fail-closed lock until the last descriptor closes.
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
#[path = "coordinator_lock_tests.rs"]
mod tests;
