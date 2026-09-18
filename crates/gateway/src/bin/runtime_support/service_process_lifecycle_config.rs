// SPDX-License-Identifier: MIT

//! Server-owned configuration for the attached process-lifecycle route surface.
//!
//! Every value here is gateway configuration, never caller input. A caller supplies only an
//! opaque numeric profile id; the executable, install, image, user-data namespace, and process
//! policy are resolved from the catalog this module builds. No path, command, URL, environment,
//! or user-data location is representable, so a configuration mistake cannot become a
//! caller-reachable arbitrary-execution surface.

use std::collections::BTreeSet;
use std::path::PathBuf;

use sts2_gateway::{
    ApprovedLaunchProfiles, ExecutableIdentity, LaunchProfile, LaunchProfileId,
    LifecycleRecordStore, ProcessPolicy, UserDataConfig,
};

/// Largest number of approved profiles one deployment may declare.
pub(super) const MAX_CONFIGURED_PROFILES: usize = 64;
/// Largest process-capacity budget one deployment may declare.
pub(super) const MAX_CONFIGURED_PROCESSES: usize = 64;
/// Largest durable record budget one deployment may declare.
pub(super) const MAX_CONFIGURED_RECORDS: usize = 1_024;

/// One configured launch profile, expressed only as the bounded numeric identity ADR 0024 defines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ConfiguredLaunchProfile {
    pub(super) profile_id: u64,
    pub(super) install_id: u64,
    pub(super) executable_id: u64,
    pub(super) image_id: u64,
    pub(super) user_data_namespace_id: u64,
    pub(super) max_descendants: usize,
    pub(super) start_timeout_millis: u64,
    pub(super) stop_timeout_millis: u64,
}

impl ConfiguredLaunchProfile {
    /// Resolves the configured numbers through the component's own validating constructors.
    ///
    /// Bounds are enforced by `ProcessPolicy`/`ExecutableIdentity`/`UserDataConfig` rather than
    /// re-implemented here, so a configuration cannot admit a value the coordinator would reject.
    pub(super) fn to_profile(self) -> Result<LaunchProfile, String> {
        LaunchProfile::try_new(
            LaunchProfileId::try_new(self.profile_id)
                .map_err(|error| format!("profile id is invalid: {error:?}"))?,
            ExecutableIdentity::try_new(self.install_id, self.executable_id, self.image_id)
                .map_err(|error| format!("executable identity is invalid: {error:?}"))?,
            UserDataConfig::try_new(self.user_data_namespace_id)
                .map_err(|error| format!("user-data namespace is invalid: {error:?}"))?,
            ProcessPolicy::try_new(
                self.max_descendants,
                self.start_timeout_millis,
                self.stop_timeout_millis,
            )
            .map_err(|error| format!("process policy is invalid: {error:?}"))?,
        )
        .map_err(|error| format!("launch profile is invalid: {error:?}"))
    }

    /// Parses `id:install:executable:image:namespace:descendants:start_ms:stop_ms`.
    fn parse(entry: &str) -> Result<Self, String> {
        let fields: Vec<&str> = entry.split(':').collect();
        if fields.len() != 8 {
            return Err(format!(
                "launch profile needs 8 colon-separated fields, found {}",
                fields.len()
            ));
        }
        let number = |index: usize, name: &str| -> Result<u64, String> {
            let text = fields[index];
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!("{name} must be a non-negative decimal integer"));
            }
            text.parse::<u64>()
                .map_err(|_| format!("{name} does not fit the supported range"))
        };
        let descendants = number(5, "max_descendants")?;
        Ok(Self {
            profile_id: number(0, "profile id")?,
            install_id: number(1, "install id")?,
            executable_id: number(2, "executable id")?,
            image_id: number(3, "image id")?,
            user_data_namespace_id: number(4, "user-data namespace id")?,
            max_descendants: usize::try_from(descendants)
                .map_err(|_| String::from("max_descendants does not fit the supported range"))?,
            start_timeout_millis: number(6, "start timeout")?,
            stop_timeout_millis: number(7, "stop timeout")?,
        })
    }
}

/// Parses the configured profile list, rejecting the whole configuration on any invalid entry.
pub(super) fn parse_profiles(specification: &str) -> Result<Vec<ConfiguredLaunchProfile>, String> {
    if specification.trim().is_empty() {
        return Err(String::from(
            "at least one approved launch profile is required",
        ));
    }
    let mut profiles = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in specification.split(';') {
        if entry.trim().is_empty() {
            return Err(String::from("launch profile list contains an empty entry"));
        }
        let profile = ConfiguredLaunchProfile::parse(entry.trim())?;
        // Resolve every entry through the component's own constructors here, so `parse_profiles`
        // is the single gate that rejects a zero id, a zero identity field, or an out-of-range
        // policy before a catalog is ever built.
        profile.to_profile()?;
        if !seen.insert(profile.profile_id) {
            return Err(format!(
                "launch profile {} is declared more than once",
                profile.profile_id
            ));
        }
        profiles.push(profile);
        if profiles.len() > MAX_CONFIGURED_PROFILES {
            return Err(format!(
                "at most {MAX_CONFIGURED_PROFILES} approved launch profiles may be declared"
            ));
        }
    }
    Ok(profiles)
}

/// Builds the server-owned catalog, rejecting the whole configuration on any invalid entry.
///
/// A partially valid catalog would silently change which profiles exist between restarts, so an
/// invalid entry fails startup instead of being skipped. The returned ids are the only profile
/// identities the capability route may advertise.
pub(super) fn build_catalog(
    configured: &[ConfiguredLaunchProfile],
) -> Result<(ApprovedLaunchProfiles, Vec<u64>), String> {
    let mut catalog = ApprovedLaunchProfiles::try_new(configured.len())
        .map_err(|error| format!("launch-profile catalog is invalid: {error:?}"))?;
    let mut ids = Vec::with_capacity(configured.len());
    for entry in configured {
        let profile = entry.to_profile()?;
        catalog.insert(profile).map_err(|error| {
            format!(
                "launch profile {} was rejected: {error:?}",
                entry.profile_id
            )
        })?;
        ids.push(entry.profile_id);
    }
    ids.sort_unstable();
    Ok((catalog, ids))
}

/// Complete deployment configuration for the attached lifecycle surface.
pub(super) struct ProcessLifecycleDeployment {
    pub(super) profiles: Vec<ConfiguredLaunchProfile>,
    pub(super) store_path: PathBuf,
    pub(super) max_processes: usize,
    pub(super) max_records: usize,
}

impl ProcessLifecycleDeployment {
    /// Confirms the durable store can be opened and integrity-checked before startup completes.
    ///
    /// The coordinator persists an intent before any process effect and replays retained
    /// operations after restart. A store that cannot be opened would therefore silently convert
    /// idempotent replay into duplicate effects, so an unusable path fails startup.
    pub(super) fn validate_store(&self) -> Result<(), String> {
        let store = sts2_gateway::SqliteLifecycleStore::open(self.store_path())
            .map_err(|error| format!("process-lifecycle store open failed: {error:?}"))?;
        store
            .count()
            .map(|_| ())
            .map_err(|error| format!("process-lifecycle store read failed: {error:?}"))
    }

    /// The configured durable store location, consumed only by composition.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn store_path(&self) -> &PathBuf {
        &self.store_path
    }
}

/// Reads the deployment configuration, or reports that the surface is not configured.
///
/// Absence is the fail-closed default: an unconfigured deployment composes no coordinator and
/// every lifecycle route refuses before any effect.
pub(super) fn from_environment() -> Result<Option<ProcessLifecycleDeployment>, String> {
    let enabled = match std::env::var("STS2_PROCESS_LIFECYCLE_ENABLED") {
        Ok(text) => match text.trim() {
            "true" => true,
            "false" => false,
            _ => {
                return Err(String::from(
                    "STS2_PROCESS_LIFECYCLE_ENABLED must be true or false",
                ));
            }
        },
        Err(std::env::VarError::NotPresent) => false,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_ENABLED is not valid UTF-8",
            ));
        }
    };
    if !enabled {
        return Ok(None);
    }
    let profiles = match std::env::var("STS2_PROCESS_LIFECYCLE_PROFILES") {
        Ok(text) => parse_profiles(&text)?,
        Err(std::env::VarError::NotPresent) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_PROFILES is required when the lifecycle surface is enabled",
            ));
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_PROFILES is not valid UTF-8",
            ));
        }
    };
    let store_path = match std::env::var("STS2_PROCESS_LIFECYCLE_STORE") {
        Ok(text) if !text.trim().is_empty() => PathBuf::from(text),
        Ok(_) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_STORE must be a non-empty path",
            ));
        }
        Err(std::env::VarError::NotPresent) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_STORE is required when the lifecycle surface is enabled",
            ));
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(String::from(
                "STS2_PROCESS_LIFECYCLE_STORE is not valid UTF-8",
            ));
        }
    };
    let max_processes = bounded(
        "STS2_PROCESS_LIFECYCLE_MAX_PROCESSES",
        MAX_CONFIGURED_PROCESSES,
    )?;
    let max_records = bounded("STS2_PROCESS_LIFECYCLE_MAX_RECORDS", MAX_CONFIGURED_RECORDS)?;
    Ok(Some(ProcessLifecycleDeployment {
        profiles,
        store_path,
        max_processes,
        max_records,
    }))
}

fn bounded(name: &str, limit: usize) -> Result<usize, String> {
    let text = match std::env::var(name) {
        Ok(text) => text,
        Err(std::env::VarError::NotPresent) => return Ok(limit),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(format!("{name} is not valid UTF-8"));
        }
    };
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{name} must be a positive decimal integer"));
    }
    let value = text
        .parse::<usize>()
        .map_err(|_| format!("{name} does not fit the supported range"))?;
    if value == 0 || value > limit {
        return Err(format!("{name} must be between 1 and {limit}"));
    }
    Ok(value)
}
