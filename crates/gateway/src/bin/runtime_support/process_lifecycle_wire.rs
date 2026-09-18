// SPDX-License-Identifier: MIT

//! Fixed authenticated routes and the versioned request schema for the process lifecycle.
//!
//! The surface is deliberately thin and closed. A caller supplies an operation id, the authority
//! epoch it believes is current, and one action. Instance, caller, session, lease, and lease epoch
//! are never taken from the body: they come from the already-authenticated request headers and the
//! gateway's own configuration, so a body cannot assert an identity the request did not prove.
//!
//! Attach is expressed as the exact process identity a caller previously received. It is not an
//! adoption mechanism: the coordinator independently compares it against the identity it retained
//! for an earlier operation, and a caller cannot express a foreign instance id at all.

use serde::Deserialize;
use sts2_gateway::{
    ExecutableIdentity, InstanceId, LifecycleAction, LifecycleError, ProcessHandle,
    ProcessIdentity, StopMode, UserDataConfig,
};

use super::json_error;

/// `POST /v1/instances/{instance_id}/process-lifecycle/operations`
pub(super) const OPERATIONS_SUFFIX: &str = "process-lifecycle/operations";
/// `GET /v1/instances/{instance_id}/process-lifecycle`
pub(super) const CAPABILITY_SUFFIX: &str = "process-lifecycle";
/// Longest accepted decimal operation id.
pub(super) const MAX_OPERATION_ID_DIGITS: usize = 20;
/// Largest accepted lifecycle request body.
pub(super) const MAX_LIFECYCLE_REQUEST_BYTES: usize = 8 * 1024;
/// Versioned contract identifier reported by capability and echoed by submissions.
pub(super) const LIFECYCLE_CONTRACT: &str = "sts2-gateway-process-lifecycle-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LifecycleRoute {
    /// Submit one lifecycle action.
    Submit,
    /// Read back a retained operation by its identity.
    Lookup,
    /// Report capability, approved profile ids, and the current authority epoch.
    Capability,
}

impl LifecycleRoute {
    pub(super) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v1/instances/{instance_id}/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("POST", OPERATIONS_SUFFIX) => Some(Self::Submit),
            ("GET", CAPABILITY_SUFFIX) => Some(Self::Capability),
            ("GET", _) => suffix
                .strip_prefix(OPERATIONS_SUFFIX)
                .and_then(|rest| rest.strip_prefix('/'))
                .filter(|id| safe_operation_id(id))
                .map(|_| Self::Lookup),
            _ => None,
        }
    }

    /// Mutations require the mutate scope; both reads require the read scope.
    pub(super) const fn is_read(self) -> bool {
        matches!(self, Self::Lookup | Self::Capability)
    }

    /// Extracts the operation id from a `Lookup` path.
    pub(super) fn operation_id(self, path: &str, instance_id: &str) -> Option<u64> {
        if self != Self::Lookup {
            return None;
        }
        let prefix = format!("/v1/instances/{instance_id}/{OPERATIONS_SUFFIX}/");
        path.strip_prefix(&prefix)?.parse::<u64>().ok()
    }
}

/// Bounded decimal operation id; rejects signs, whitespace, and leading-zero aliasing.
fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPERATION_ID_DIGITS
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

/// A lifecycle submission. Identity comes from the authenticated request, not this body.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LifecycleSubmission {
    operation_id: u64,
    authority_epoch: u64,
    action: LifecycleSubmissionAction,
}

impl LifecycleSubmission {
    pub(super) fn parse(body: &[u8]) -> Result<Self, (u16, Vec<u8>)> {
        if body.len() > MAX_LIFECYCLE_REQUEST_BYTES {
            return Err((413, json_error("process_lifecycle_request_too_large")));
        }
        let submission: Self = serde_json::from_slice(body)
            .map_err(|_| (400, json_error("process_lifecycle_request_invalid")))?;
        if submission.operation_id == 0 {
            return Err((400, json_error("process_lifecycle_operation_id_invalid")));
        }
        if submission.authority_epoch == 0 {
            return Err((400, json_error("process_lifecycle_authority_epoch_invalid")));
        }
        Ok(submission)
    }

    pub(super) const fn operation_id(&self) -> u64 {
        self.operation_id
    }

    pub(super) const fn authority_epoch(&self) -> u64 {
        self.authority_epoch
    }

    /// Resolves the closed action against the gateway's own instance identity.
    pub(super) fn resolve_action(
        &self,
        instance_id: InstanceId,
    ) -> Result<LifecycleAction, (u16, Vec<u8>)> {
        self.action.resolve_action(instance_id)
    }
}

/// One action in the versioned request schema.
///
/// Fields for other actions are rejected rather than ignored, so a body cannot smuggle an
/// alternate action past the parser or silently change meaning when the schema grows.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LifecycleSubmissionAction {
    kind: String,
    profile_id: Option<u64>,
    mode: Option<String>,
    process: Option<u64>,
    pid: Option<u64>,
    birth_id: Option<u64>,
    executable: Option<SubmissionExecutable>,
    user_data: Option<SubmissionUserData>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionExecutable {
    install_id: u64,
    executable_id: u64,
    image_id: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionUserData {
    namespace_id: u64,
}

impl LifecycleSubmissionAction {
    fn resolve_action(&self, instance_id: InstanceId) -> Result<LifecycleAction, (u16, Vec<u8>)> {
        let malformed = || (400, json_error("process_lifecycle_action_invalid"));
        let profile =
            |value: Option<u64>| -> Result<sts2_gateway::LaunchProfileId, (u16, Vec<u8>)> {
                let raw = value.ok_or_else(malformed)?;
                sts2_gateway::LaunchProfileId::try_new(raw).map_err(|_| malformed())
            };
        match self.kind.as_str() {
            "launch_new" if self.only(&["profile_id"]) => Ok(LifecycleAction::LaunchNew {
                profile_id: profile(self.profile_id)?,
            }),
            "restart" if self.only(&["profile_id"]) => Ok(LifecycleAction::Restart {
                profile_id: profile(self.profile_id)?,
            }),
            "stop" if self.only(&["mode"]) => {
                let mode = match self.mode.as_deref() {
                    Some("graceful") => StopMode::Graceful,
                    Some("force") => StopMode::Force,
                    _ => return Err(malformed()),
                };
                Ok(LifecycleAction::Stop { mode })
            }
            "attach_existing"
                if self.only(&["process", "pid", "birth_id", "executable", "user_data"]) =>
            {
                Ok(LifecycleAction::AttachExisting {
                    identity: self.identity(instance_id).ok_or_else(malformed)?,
                })
            }
            _ => Err(malformed()),
        }
    }

    /// True when every present optional field is named in `allowed`.
    ///
    /// Fields belonging to another action are rejected rather than ignored, so one body can never
    /// be two actions at once and a caller cannot smuggle an attach identity into a launch.
    fn only(&self, allowed: &[&str]) -> bool {
        let present = [
            ("profile_id", self.profile_id.is_some()),
            ("mode", self.mode.is_some()),
            ("process", self.process.is_some()),
            ("pid", self.pid.is_some()),
            ("birth_id", self.birth_id.is_some()),
            ("executable", self.executable.is_some()),
            ("user_data", self.user_data.is_some()),
        ];
        present
            .iter()
            .all(|(name, is_present)| !is_present || allowed.contains(name))
    }

    /// Builds the exact identity a caller echoes, with the instance id taken from configuration.
    fn identity(&self, instance_id: InstanceId) -> Option<ProcessIdentity> {
        let executable = self.executable?;
        let user_data = self.user_data?;
        Some(ProcessIdentity::new(
            instance_id,
            ProcessHandle::new(self.process?),
            self.pid?,
            self.birth_id?,
            ExecutableIdentity::try_new(
                executable.install_id,
                executable.executable_id,
                executable.image_id,
            )
            .ok()?,
            UserDataConfig::try_new(user_data.namespace_id).ok()?,
        ))
    }
}

/// Maps a coordinator failure onto a stable HTTP status and machine-readable code.
///
/// Every arm is fail-closed and typed. A caller can distinguish a stale epoch, a fence rejection,
/// an unowned attach, and an unavailable profile without reading a message string, and no arm
/// converts a failure into an invented success or a retried mutation.
pub(super) fn lifecycle_error(error: &LifecycleError) -> (u16, Vec<u8>) {
    let (status, code) = match error {
        LifecycleError::LeaseNotBound => (409, "process_lifecycle_lease_not_bound"),
        LifecycleError::Fence(_) => (409, "process_lifecycle_fence_rejected"),
        LifecycleError::StaleAuthorityEpoch => (409, "process_lifecycle_authority_epoch_stale"),
        LifecycleError::AuthorityExhausted => (409, "process_lifecycle_authority_exhausted"),
        LifecycleError::CapacityExceeded => (429, "process_lifecycle_capacity_exceeded"),
        LifecycleError::InstanceBusy => (409, "process_lifecycle_instance_busy"),
        LifecycleError::UserDataNamespaceBusy => (409, "process_lifecycle_namespace_busy"),
        LifecycleError::InstanceNotFound => (404, "process_lifecycle_instance_not_found"),
        LifecycleError::OperationConflict => (409, "process_lifecycle_operation_conflict"),
        LifecycleError::OperationNotFound => (404, "process_lifecycle_operation_not_found"),
        LifecycleError::UnownedAttach => (403, "process_lifecycle_unowned_attach"),
        LifecycleError::IdentityMismatch => (409, "process_lifecycle_identity_mismatch"),
        LifecycleError::ForeignDescendant => (409, "process_lifecycle_foreign_descendant"),
        LifecycleError::Profile(_) => (422, "process_lifecycle_profile_rejected"),
        LifecycleError::Process(_) => (503, "process_lifecycle_process_unavailable"),
        LifecycleError::Store(_) => (503, "process_lifecycle_store_unavailable"),
        LifecycleError::InvalidState(_) => (409, "process_lifecycle_invalid_state"),
    };
    (status, json_error(code))
}
