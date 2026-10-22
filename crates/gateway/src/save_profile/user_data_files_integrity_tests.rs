// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn malformed_oversized_and_foreign_provenance_is_preserved_and_refused() -> Result<(), String> {
    for case in 0..6 {
        let root = TestRoot::new();
        let mut port = root.port().map_err(|error| format!("{error:?}"))?;
        port.create(request(1, "op-1")?)
            .map_err(|error| format!("{error:?}"))?;
        let path = root.0.join("run-1").join(PROVENANCE_FILE);
        let original = fs::read(&path).map_err(|error| error.to_string())?;
        let mut value: serde_json::Value =
            serde_json::from_slice(&original).map_err(|error| error.to_string())?;
        let bytes = match case {
            0 => vec![b' '; 4097],
            1 => original
                .strip_prefix(b"{")
                .map(|tail| [b"{\"identity\":1,".as_slice(), tail].concat())
                .ok_or("missing object")?,
            2 => {
                value["unknown"] = serde_json::json!(true);
                serde_json::to_vec(&value).map_err(|error| error.to_string())?
            }
            3 => {
                value["provenance"]["owner"] = serde_json::json!("foreign");
                serde_json::to_vec(&value).map_err(|error| error.to_string())?
            }
            4 => {
                value["provenance"]["contract"] = serde_json::json!("foreign-v1");
                serde_json::to_vec(&value).map_err(|error| error.to_string())?
            }
            _ => {
                value["identity"] = serde_json::json!(2);
                serde_json::to_vec(&value).map_err(|error| error.to_string())?
            }
        };
        fs::write(&path, &bytes).map_err(|error| error.to_string())?;
        assert_eq!(
            port.inspect(UserDataIdentity::new(1)),
            Err(UserDataPortError::UnknownContents),
            "case {case}"
        );
        assert_eq!(
            port.create(request(1, "op-1")?),
            Err(UserDataPortError::UnknownContents)
        );
        assert_eq!(fs::read(&path).map_err(|error| error.to_string())?, bytes);
        assert!(!root.0.join("run-2").exists());
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn provenance_symlinks_and_hardlinks_never_become_ownership() -> Result<(), String> {
    use std::os::unix::fs::symlink;
    for hardlink in [false, true] {
        let root = TestRoot::new();
        let outside = TestRoot::new();
        let mut port = root.port().map_err(|error| format!("{error:?}"))?;
        port.create(request(1, "op-1")?)
            .map_err(|error| format!("{error:?}"))?;
        let path = root.0.join("run-1").join(PROVENANCE_FILE);
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        let external = outside.0.join("foreign-record.json");
        if hardlink {
            fs::hard_link(&path, &external).map_err(|error| error.to_string())?;
        } else {
            fs::write(&external, &bytes).map_err(|error| error.to_string())?;
            fs::remove_file(&path).map_err(|error| error.to_string())?;
            symlink(&external, &path).map_err(|error| error.to_string())?;
        }
        let expected = if hardlink {
            UserDataPortError::UnknownContents
        } else {
            UserDataPortError::SymlinkEscape
        };
        assert_eq!(port.inspect(UserDataIdentity::new(1)), Err(expected));
        assert_eq!(port.create(request(1, "op-1")?), Err(expected));
        assert_eq!(
            fs::read(&external).map_err(|error| error.to_string())?,
            bytes
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn fresh_allocation_and_provenance_have_private_permissions() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let root = TestRoot::new();
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    port.create(request(1, "op-1")?)
        .map_err(|error| format!("{error:?}"))?;
    for (path, mode) in [
        (root.0.join("run-1"), 0o700),
        (root.0.join("run-1").join(PROVENANCE_FILE), 0o600),
    ] {
        assert_eq!(
            fs::metadata(path)
                .map_err(|error| error.to_string())?
                .permissions()
                .mode()
                & 0o777,
            mode
        );
    }
    Ok(())
}

#[test]
fn non_file_provenance_and_missing_root_are_not_owned_or_absent() -> Result<(), String> {
    let root = TestRoot::new();
    let mut port = root.port().map_err(|error| format!("{error:?}"))?;
    fs::create_dir(root.0.join("run-1")).map_err(|error| error.to_string())?;
    fs::create_dir(root.0.join("run-1").join(PROVENANCE_FILE))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        port.inspect(UserDataIdentity::new(1)),
        Err(UserDataPortError::UnknownContents)
    );
    fs::remove_dir_all(&root.0).map_err(|error| error.to_string())?;
    assert_eq!(
        port.inspect(UserDataIdentity::new(1)),
        Err(UserDataPortError::Unavailable)
    );
    Ok(())
}
