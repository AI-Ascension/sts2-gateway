// SPDX-License-Identifier: MIT

#![cfg(unix)]

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use super::CoordinatorLock;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> io::Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "sts2-coordinator-lock-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&directory)?;
        Ok(Self(directory))
    }

    fn path(&self) -> PathBuf {
        self.0.join("owner.lock")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

fn refuses_competitor(path: &Path) -> io::Result<()> {
    let contender = open(path)?;
    let result = FileExt::try_lock_exclusive(&contender);
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::WouldBlock));
    Ok(())
}

#[test]
fn closing_a_descriptor_alone_retains_ownership_through_a_duplicate() -> io::Result<()> {
    let fixture = Fixture::new()?;
    let owner = open(&fixture.path())?;
    FileExt::try_lock_exclusive(&owner)?;
    let inherited = owner.try_clone()?;
    drop(owner);
    refuses_competitor(&fixture.path())?;
    drop(inherited);
    let next = CoordinatorLock::acquire(open(&fixture.path())?)?;
    drop(next);
    Ok(())
}

#[test]
fn guard_drop_releases_ownership_while_an_inherited_description_remains_open() -> io::Result<()> {
    let fixture = Fixture::new()?;
    let owner = CoordinatorLock::acquire(open(&fixture.path())?)?;
    let inherited = owner.file.try_clone()?;
    refuses_competitor(&fixture.path())?;
    drop(owner);
    let next = CoordinatorLock::acquire(open(&fixture.path())?)?;
    refuses_competitor(&fixture.path())?;
    drop(inherited);
    // Closing the stale descriptor must not release the new owner's independent lock.
    refuses_competitor(&fixture.path())?;
    drop(next);
    Ok(())
}
