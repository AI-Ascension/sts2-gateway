// SPDX-License-Identifier: MIT

use super::{LaunchProfile, NativeProcessError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[path = "process_native_config_io.rs"]
mod io;
use io::executable_file;
pub(super) use io::{
    bounded_read, exact_path, hex_digest, image_digest, private_directory, valid_digest,
};

const MAX_CONFIG_BYTES: u64 = 262_144;
const MARKER: &str = ".sts2-gateway-process-owner";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentFile {
    schema_version: String,
    registry_path: PathBuf,
    runtime_directory: PathBuf,
    systemctl_path: PathBuf,
    systemd_run_path: PathBuf,
    profiles: Vec<ProfileFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    profile_id: u64,
    install_root: PathBuf,
    executable: PathBuf,
    executable_sha256: String,
    user_data_root: PathBuf,
    arguments: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct NativeProfile {
    pub(super) profile: LaunchProfile,
    pub(super) executable: PathBuf,
    pub(super) image_digest: String,
    pub(super) user_data: PathBuf,
    pub(super) arguments: Vec<String>,
    pub(super) digest: String,
}

pub(super) struct NativeConfiguration {
    pub(super) registry_path: PathBuf,
    pub(super) runtime_directory: PathBuf,
    pub(super) systemctl_path: PathBuf,
    pub(super) systemd_run_path: PathBuf,
    pub(super) profiles: BTreeMap<u64, NativeProfile>,
}

impl NativeConfiguration {
    pub(super) fn read(
        path: &Path,
        approved: &[LaunchProfile],
    ) -> Result<Self, NativeProcessError> {
        exact_path(path, false)?;
        let bytes = bounded_read(path, MAX_CONFIG_BYTES)?;
        let file: DeploymentFile =
            serde_json::from_slice(&bytes).map_err(|_| NativeProcessError::Configuration)?;
        if file.schema_version != "sts2-gateway-systemd-process/v1"
            || approved.is_empty()
            || approved.len() > 64
            || file.profiles.len() != approved.len()
        {
            return Err(NativeProcessError::Configuration);
        }
        exact_path(&file.runtime_directory, true)?;
        private_directory(&file.runtime_directory)?;
        exact_path(&file.systemctl_path, false)?;
        exact_path(&file.systemd_run_path, false)?;
        executable_file(&file.systemctl_path)?;
        executable_file(&file.systemd_run_path)?;
        let parent = file
            .registry_path
            .parent()
            .ok_or(NativeProcessError::Configuration)?;
        exact_path(parent, true)?;
        private_directory(parent)?;
        let mut profiles = BTreeMap::new();
        let mut user_data = BTreeSet::<PathBuf>::new();
        let mut namespaces = BTreeSet::new();
        for entry in file.profiles {
            let profile = approved
                .iter()
                .find(|profile| profile.id().value() == entry.profile_id)
                .copied()
                .ok_or(NativeProcessError::Profile)?;
            let binding = NativeProfile::resolve(entry, profile, &file.registry_path)?;
            if user_data.iter().any(|root| {
                binding.user_data.starts_with(root) || root.starts_with(&binding.user_data)
            }) || !namespaces.insert(profile.user_data().namespace_id())
            {
                return Err(NativeProcessError::Profile);
            }
            user_data.insert(binding.user_data.clone());
            if profiles.insert(profile.id().value(), binding).is_some() {
                return Err(NativeProcessError::Profile);
            }
        }
        let configuration = Self {
            registry_path: file.registry_path,
            runtime_directory: file.runtime_directory,
            systemctl_path: file.systemctl_path,
            systemd_run_path: file.systemd_run_path,
            profiles,
        };
        configuration.protect_paths(&[
            path,
            &configuration.registry_path,
            &configuration.runtime_directory,
            &configuration.systemctl_path,
            &configuration.systemd_run_path,
        ])?;
        for profile in configuration.profiles.values() {
            configuration.protect_paths(&[&profile.executable])?;
        }
        Ok(configuration)
    }

    pub(super) fn protect_paths(&self, paths: &[&Path]) -> Result<(), NativeProcessError> {
        for path in paths {
            let resolved = io::protected_path(path)?;
            if self
                .profiles
                .values()
                .any(|profile| resolved.starts_with(&profile.user_data))
            {
                return Err(NativeProcessError::Profile);
            }
        }
        Ok(())
    }
}

impl NativeProfile {
    fn resolve(
        entry: ProfileFile,
        profile: LaunchProfile,
        registry: &Path,
    ) -> Result<Self, NativeProcessError> {
        exact_path(&entry.install_root, true)?;
        exact_path(&entry.executable, false)?;
        executable_file(&entry.executable)?;
        exact_path(&entry.user_data_root, true)?;
        private_directory(&entry.user_data_root)?;
        if !entry.executable.starts_with(&entry.install_root)
            || !valid_digest(&entry.executable_sha256)
            || image_digest(&entry.executable)? != entry.executable_sha256
            || entry.arguments.len() > 64
            || entry
                .arguments
                .iter()
                .any(|arg| arg.len() > 1024 || arg.contains(['\0', '\n', '\r', '%']))
        {
            return Err(NativeProcessError::Profile);
        }
        let digest = hex_digest(
            &serde_json::to_vec(&(profile, &entry, registry))
                .map_err(|_| NativeProcessError::Configuration)?,
        );
        let user_data_text = entry
            .user_data_root
            .to_str()
            .ok_or(NativeProcessError::Profile)?;
        let arguments = entry
            .arguments
            .iter()
            .map(|arg| arg.replace("{user_data}", user_data_text))
            .collect();
        Ok(Self {
            profile,
            executable: entry.executable,
            image_digest: entry.executable_sha256,
            user_data: entry.user_data_root,
            arguments,
            digest,
        })
    }

    pub(super) fn verify_image(&self) -> Result<(), NativeProcessError> {
        exact_path(&self.executable, false)?;
        executable_file(&self.executable)?;
        if image_digest(&self.executable)? != self.image_digest {
            return Err(NativeProcessError::Profile);
        }
        Ok(())
    }

    pub(super) fn claim_user_data(
        &self,
        registry_identity: &str,
    ) -> Result<(), NativeProcessError> {
        exact_path(&self.user_data, true)?;
        private_directory(&self.user_data)?;
        let marker = self.user_data.join(MARKER);
        let owner = hex_digest(format!("{}:{registry_identity}", self.digest).as_bytes());
        if marker.exists() {
            exact_path(&marker, false)?;
            if bounded_read(&marker, 64)? == owner.as_bytes() {
                return Ok(());
            }
            return Err(NativeProcessError::Profile);
        }
        if fs::read_dir(&self.user_data)
            .map_err(|_| NativeProcessError::Profile)?
            .next()
            .is_some()
        {
            return Err(NativeProcessError::Profile);
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&marker)
            .map_err(|_| NativeProcessError::Profile)?;
        file.write_all(owner.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|_| NativeProcessError::Profile)?;
        File::open(&self.user_data)
            .and_then(|file| file.sync_all())
            .map_err(|_| NativeProcessError::Profile)
    }
}
