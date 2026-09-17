// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;

use super::provisioning_types::{
    UserDataCreateOutcome, UserDataCreateRequest, UserDataInspection, UserDataPort,
    UserDataPortError,
};
use super::types::{
    LAUNCH_PROFILE_CONTRACT, LAUNCH_PROFILE_ID, LaunchProfileBinding, UserDataDescriptor,
    UserDataIdentity, UserDataProvenance,
};
use super::user_data_files::FilesystemUserDataPort;
use super::user_data_files_validation::PROVENANCE_FILE;
use uuid::Uuid;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sts2-gw-root-{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&path);
        Self(path)
    }

    fn port(&self) -> Result<FilesystemUserDataPort, UserDataPortError> {
        FilesystemUserDataPort::try_new(self.0.clone(), 8)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn descriptor(identity: u64, operation_id: &str) -> UserDataDescriptor {
    UserDataDescriptor {
        identity: UserDataIdentity::new(identity),
        provenance: UserDataProvenance {
            owner: String::from(super::provisioning_descriptor::PROVENANCE_OWNER),
            instance_id: String::from("instance-1"),
            operation_id: operation_id.to_owned(),
            contract: LAUNCH_PROFILE_CONTRACT.to_owned(),
        },
        baseline: None,
    }
}

fn request(identity: u64, operation_id: &str) -> Result<UserDataCreateRequest, String> {
    let descriptor = descriptor(identity, operation_id);
    Ok(UserDataCreateRequest {
        launch_profile: LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, descriptor.identity)
            .map_err(|error| format!("{error:?}"))?,
        descriptor,
    })
}

#[test]
fn fresh_allocation_creates_only_the_opaque_directory() -> Result<(), String> {
    let root = TestRoot::new();
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        port.inspect(UserDataIdentity::new(1))
            .map_err(|error| format!("{error:?}"))?,
        UserDataInspection::Absent
    );
    let outcome = port
        .create(request(1, "op-1")?)
        .map_err(|error| format!("{error:?}"))?;
    assert!(matches!(outcome, UserDataCreateOutcome::Created(_)));
    let owned = match port
        .inspect(UserDataIdentity::new(1))
        .map_err(|error| format!("{error:?}"))?
    {
        UserDataInspection::Owned(provenance) => provenance,
        other => return Err(format!("unexpected inspection: {other:?}")),
    };
    assert_eq!(owned.operation_id, "op-1");
    let entries = fs::read_dir(root.0.join("run-1")).map_err(|error| error.to_string())?;
    let names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert_eq!(names, vec![String::from(PROVENANCE_FILE)]);
    Ok(())
}

#[test]
fn same_identity_replays_without_a_second_directory() -> Result<(), String> {
    let root = TestRoot::new();
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    let first = port
        .create(request(1, "op-1")?)
        .map_err(|error| format!("{error:?}"))?;
    let second = port
        .create(request(1, "op-1")?)
        .map_err(|error| format!("{error:?}"))?;
    assert!(matches!(first, UserDataCreateOutcome::Created(_)));
    assert!(matches!(second, UserDataCreateOutcome::AlreadyCreated(_)));
    let mut entries = fs::read_dir(&root.0).map_err(|error| error.to_string())?;
    assert!(entries.next().is_some());
    assert!(entries.next().is_none(), "no extra allocation appeared");
    Ok(())
}

#[test]
fn unknown_existing_contents_and_foreign_provenance_are_refused() -> Result<(), String> {
    let root = TestRoot::new();
    fs::create_dir_all(root.0.join("run-1")).map_err(|error| error.to_string())?;
    fs::write(root.0.join("run-1").join("user-save.dat"), b"private")
        .map_err(|error| error.to_string())?;
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        port.inspect(UserDataIdentity::new(1))
            .map_err(|error| format!("{error:?}"))?,
        UserDataInspection::UnknownContents
    );
    assert!(matches!(
        port.create(request(1, "op-1")?),
        Err(UserDataPortError::ExistingContents)
    ));
    assert!(
        root.0.join("run-1").join("user-save.dat").exists(),
        "refused contents must be preserved"
    );

    let empty = root.0.join("run-2");
    fs::create_dir_all(&empty).map_err(|error| error.to_string())?;
    assert!(matches!(
        port.create(request(2, "op-2")?),
        Err(UserDataPortError::ExistingContents)
    ));
    assert!(empty.exists(), "pre-existing directory must be preserved");
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_allocation_is_refused_as_an_escape() -> Result<(), String> {
    use std::os::unix::fs::symlink;
    let root = TestRoot::new();
    let outside = TestRoot::new();
    symlink(&outside.0, root.0.join("run-1")).map_err(|error| error.to_string())?;
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    assert!(matches!(
        port.inspect(UserDataIdentity::new(1)),
        Err(UserDataPortError::SymlinkEscape)
    ));
    assert!(matches!(
        port.create(request(1, "op-1")?),
        Err(UserDataPortError::SymlinkEscape)
    ));
    assert!(
        fs::read_dir(&outside.0)
            .map_err(|error| error.to_string())?
            .next()
            .is_none(),
        "no write may cross the symlink"
    );
    Ok(())
}

#[test]
fn identity_outside_the_bounded_root_is_refused() -> Result<(), String> {
    let root = TestRoot::new();
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    assert!(matches!(
        port.inspect(UserDataIdentity::new(9)),
        Err(UserDataPortError::Traversal)
    ));
    assert!(FilesystemUserDataPort::try_new(PathBuf::from("relative/root"), 4).is_err());
    assert!(FilesystemUserDataPort::try_new(&root.0, 0).is_err());
    Ok(())
}
