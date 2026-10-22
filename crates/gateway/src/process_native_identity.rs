// SPDX-License-Identifier: MIT

use super::configuration::{bounded_read, image_digest};
use super::{NativeMember, NativeProcessError, ProcessFault};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(super) fn boot_id() -> Result<String, NativeProcessError> {
    if !cfg!(target_os = "linux") || !Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
        return Err(NativeProcessError::UnsupportedHost);
    }
    let bytes = bounded_read(Path::new("/proc/sys/kernel/random/boot_id"), 64)?;
    let value = std::str::from_utf8(&bytes)
        .map_err(|_| NativeProcessError::UnsupportedHost)?
        .trim();
    uuid::Uuid::parse_str(value).map_err(|_| NativeProcessError::UnsupportedHost)?;
    Ok(value.to_owned())
}

pub(super) fn valid_group(path: &str, unit: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4096
        && !path.contains(['\0', '\n', '\r'])
        && !path.contains("//")
        && !path.split('/').any(|part| matches!(part, "." | ".."))
        && Path::new(path).file_name().and_then(|name| name.to_str()) == Some(unit)
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn group_path(path: &str) -> Result<PathBuf, ProcessFault> {
    let unit = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ProcessFault::IdentityMismatch)?;
    if !unit.starts_with("sts2-gateway-") || !unit.ends_with(".service") || !valid_group(path, unit)
    {
        return Err(ProcessFault::IdentityMismatch);
    }
    Ok(Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

pub(super) fn group_members(path: &str, limit: usize) -> Result<Vec<NativeMember>, ProcessFault> {
    let Some(directory) = existing_group(path)? else {
        return Ok(Vec::new());
    };
    let entries = std::fs::read_dir(&directory).map_err(|_| ProcessFault::InspectionFailed)?;
    for entry in entries {
        let entry = entry.map_err(|_| ProcessFault::InspectionFailed)?;
        if entry
            .file_type()
            .map_err(|_| ProcessFault::InspectionFailed)?
            .is_dir()
        {
            // Delegate=no and ProtectControlGroups=yes are required. Nested groups are not adopted.
            return Err(ProcessFault::DescendantOutOfScope);
        }
    }
    let bytes = bounded_read(&directory.join("cgroup.procs"), 8192)
        .map_err(|_| ProcessFault::InspectionFailed)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ProcessFault::InspectionFailed)?;
    let mut pids = BTreeSet::new();
    for line in text.lines() {
        let pid = line
            .parse::<u64>()
            .map_err(|_| ProcessFault::InspectionFailed)?;
        if pid == 0 || pid > u32::MAX as u64 || !pids.insert(pid) || pids.len() > limit {
            return Err(ProcessFault::DescendantOutOfScope);
        }
    }
    pids.into_iter().map(|pid| member(pid, path)).collect()
}

pub(super) fn empty_group(path: &str) -> Result<bool, ProcessFault> {
    let Some(directory) = existing_group(path)? else {
        return Ok(true);
    };
    let bytes = bounded_read(&directory.join("cgroup.events"), 1024)
        .map_err(|_| ProcessFault::InspectionFailed)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ProcessFault::InspectionFailed)?;
    match text
        .lines()
        .find_map(|line| line.strip_prefix("populated "))
    {
        Some("0") => Ok(true),
        Some("1") => Ok(false),
        _ => Err(ProcessFault::InspectionFailed),
    }
}

fn existing_group(path: &str) -> Result<Option<PathBuf>, ProcessFault> {
    let directory = group_path(path)?;
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Some(directory))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => Err(ProcessFault::InspectionFailed),
    }
}

fn member(pid: u64, group: &str) -> Result<NativeMember, ProcessFault> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    let before = birth(
        &bounded_read(&root.join("stat"), 8192).map_err(|_| ProcessFault::InspectionFailed)?,
    )?;
    verify_membership(&root, group)?;
    let image = image_digest(&root.join("exe")).map_err(|_| ProcessFault::InspectionFailed)?;
    let executable =
        std::fs::read_link(root.join("exe")).map_err(|_| ProcessFault::InspectionFailed)?;
    verify_membership(&root, group)?;
    let after = birth(
        &bounded_read(&root.join("stat"), 8192).map_err(|_| ProcessFault::InspectionFailed)?,
    )?;
    if before != after {
        return Err(ProcessFault::IdentityMismatch);
    }
    Ok(NativeMember {
        pid,
        birth: before,
        control_group: group.to_owned(),
        executable,
        image_digest: image,
    })
}

fn verify_membership(root: &Path, expected: &str) -> Result<(), ProcessFault> {
    let bytes =
        bounded_read(&root.join("cgroup"), 8192).map_err(|_| ProcessFault::InspectionFailed)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ProcessFault::InspectionFailed)?;
    let mut groups = text.lines().filter_map(|line| line.strip_prefix("0::"));
    if groups.next() != Some(expected) || groups.next().is_some() {
        return Err(ProcessFault::IdentityMismatch);
    }
    Ok(())
}

pub(super) fn birth(stat: &[u8]) -> Result<u64, ProcessFault> {
    let text = std::str::from_utf8(stat).map_err(|_| ProcessFault::InspectionFailed)?;
    // comm (field 2) may contain spaces and parentheses; fields after its final ')' are numeric/state.
    let suffix = text
        .rsplit_once(')')
        .map(|(_, suffix)| suffix)
        .ok_or(ProcessFault::InspectionFailed)?;
    let value = suffix
        .split_whitespace()
        .nth(19)
        .ok_or(ProcessFault::InspectionFailed)?
        .parse::<u64>()
        .map_err(|_| ProcessFault::InspectionFailed)?;
    if value == 0 {
        return Err(ProcessFault::InspectionFailed);
    }
    Ok(value)
}
