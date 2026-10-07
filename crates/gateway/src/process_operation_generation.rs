// SPDX-License-Identifier: MIT

use crate::{AuthorityEpoch, InstanceId, LaunchProfile, LaunchProfileId, OperationId};

use crate::process_store::{LifecycleAction, LifecycleOperation, LifecycleOperationState};

/// Distinguishes the two persisted lifecycle actions that may create a process.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProcessGenerationKind {
    /// Creates the initial process for a launch operation.
    LaunchNew,
    /// Creates a replacement after a restart operation's authority fence advances.
    RestartReplacement,
}

/// Opaque identity for one process creation attempt authorized by a persisted operation.
///
/// Start and recovery for one attempt use the same key. It does not identify a call phase or an
/// existing process; those operations remain bound to `ProcessIdentity`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProcessOperationGeneration {
    instance_id: InstanceId,
    operation_id: OperationId,
    sequence: u64,
    request_epoch: AuthorityEpoch,
    authority_epoch: AuthorityEpoch,
    profile_id: LaunchProfileId,
    kind: ProcessGenerationKind,
}

impl ProcessOperationGeneration {
    /// Checks the persisted fence before resolving a replacement profile or querying an adapter.
    pub(crate) fn permits_restart_recovery(
        operation: &LifecycleOperation,
        current_epoch: AuthorityEpoch,
    ) -> bool {
        operation.sequence() != 0
            && operation.authority_epoch() == current_epoch
            && operation.request_epoch() != operation.authority_epoch()
    }

    /// Builds a key only from a sequenced persisted operation and its resolved profile.
    pub(crate) fn for_operation(
        operation: &LifecycleOperation,
        profile: LaunchProfile,
        kind: ProcessGenerationKind,
    ) -> Option<Self> {
        let operation_profile = match (kind, operation.action()) {
            (ProcessGenerationKind::LaunchNew, LifecycleAction::LaunchNew { profile_id })
            | (
                ProcessGenerationKind::RestartReplacement,
                LifecycleAction::Restart { profile_id },
            ) => *profile_id,
            _ => return None,
        };
        let state_allows_generation = match kind {
            ProcessGenerationKind::LaunchNew => matches!(
                operation.state(),
                LifecycleOperationState::Starting | LifecycleOperationState::Unknown
            ),
            ProcessGenerationKind::RestartReplacement => matches!(
                operation.state(),
                LifecycleOperationState::Restarting | LifecycleOperationState::Unknown
            ),
        };
        if operation.sequence() == 0 || profile.id() != operation_profile {
            return None;
        }
        if !state_allows_generation {
            return None;
        }
        if kind == ProcessGenerationKind::RestartReplacement
            && operation.request_epoch() == operation.authority_epoch()
        {
            return None;
        }
        Some(Self {
            instance_id: operation.instance_id(),
            operation_id: operation.operation_id(),
            sequence: operation.sequence(),
            request_epoch: operation.request_epoch(),
            authority_epoch: operation.authority_epoch(),
            profile_id: operation_profile,
            kind,
        })
    }

    /// Returns the instance whose operation authorized this process generation.
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Returns the persisted lifecycle operation that authorized this generation.
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Returns the nonzero sequence assigned by the Gateway record store.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the epoch recorded on the original lifecycle request.
    ///
    /// This value is part of the exact lookup key; it does not authorize effects by itself.
    pub const fn request_epoch(&self) -> AuthorityEpoch {
        self.request_epoch
    }

    /// Returns the authority epoch persisted for this generation.
    ///
    /// Restart replacements use the post-fence epoch; existing-process ownership still uses
    /// `ProcessIdentity`.
    pub const fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority_epoch
    }

    /// Returns the server-resolved profile bound to the generation.
    pub const fn profile_id(&self) -> LaunchProfileId {
        self.profile_id
    }

    /// Returns whether this key represents an initial launch or restart replacement.
    pub const fn kind(&self) -> ProcessGenerationKind {
        self.kind
    }
}

/// Failure returned by a generation-aware start.
///
/// The default `ProcessPort` implementation returns [`Unsupported`](Self::Unsupported) without
/// calling legacy start methods, so existing implementations remain source-compatible but must
/// override generation methods to support lifecycle launches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationStartError {
    /// The adapter refused with a guarantee that it created or transferred no process.
    Unsupported,
    /// A start fault may be ambiguous; look up this exact key without repeating the start.
    Process(crate::ProcessFault),
}

/// Read-only result of looking up one exact process generation.
///
/// The default `ProcessPort` implementation returns [`Indeterminate`](Self::Indeterminate)
/// without calling legacy instance/profile recovery. An indeterminate result preserves the
/// operation's uncertainty and reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GenerationRecovery {
    /// The adapter-reported identity for this key; lifecycle ownership and cleanup remain
    /// identity-bound.
    Found(crate::ProcessIdentity),
    /// The adapter cannot prove whether this exact generation exists; do not start again.
    Indeterminate,
}
