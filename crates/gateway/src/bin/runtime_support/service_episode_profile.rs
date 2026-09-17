// SPDX-License-Identifier: MIT

//! Explicit, negotiated repeated-episode lease profile.
//!
//! The gateway treats an attached deployment as a single episode by default: a
//! successful `release` permanently revokes the local lease context so no later
//! allocation is admitted. The watchdog continuous-soak prerequisite
//! (AI-Ascension/sts2-gateway#67) needs two *completed* episodes in one
//! deployment without weakening that fail-closed default.
//!
//! A caller opts in with `x-sts2-episode-profile: repeated-episode-lease-v1` on
//! the release that completes an episode. The negotiation is gateway-local
//! process state, deliberately *not* written to the durable boot authority or
//! release set: the process, boot, fence, and release-set identities are
//! preserved and a fresh process starts with no profile. Once negotiated, a
//! completed-episode release clears the active lease without setting the
//! permanent stop flag; the next allocation must land on a strictly higher
//! epoch of the *same* boot authority.
//!
//! Every other stop path (operator revoke, explicit revoke, shutdown,
//! unresolved or rejected host revoke acknowledgment, stale fence, wrong
//! caller/session, restart before a release completes) keeps the permanent flag
//! set and never reopens admission. A profile whose boot authority has rotated
//! is treated as revoked rather than ignored.

use serde_json::{Value, json};
use sts2_gateway::{RecoveryBootContext, RecoveryLease};

use super::{RuntimeService, json_error};

/// Negotiated profile/capability name.
pub(super) const EPISODE_PROFILE_NAME: &str = "repeated-episode-lease-v1";
/// Capability/version identifier exposed in the release witness.
pub(super) const EPISODE_PROFILE_CAPABILITY: &str = "sts2-gateway/repeated-episode-lease-v1";
/// Header that negotiates the profile.
pub(super) const EPISODE_PROFILE_HEADER: &str = "x-sts2-episode-profile";
/// Schema digest of the gateway-owned profile descriptor: the SHA-256 of the
/// compact canonical JSON recorded immediately below (raw UTF-8 bytes), so a
/// reviewer can reproduce it without trusting this code.
///
/// ```text
/// {"admission":"explicit-header-negotiated","capability":"sts2-gateway/repeated-episode-lease-v1","header":"x-sts2-episode-profile","profile":"repeated-episode-lease-v1","reopen":"completed-episode-release","scope":"deployment","version":1}
/// ```
pub(super) const EPISODE_PROFILE_SCHEMA_DIGEST: &str =
    "f3a04bab61ce4898eda0fa88cb546441493e49eef19b1e5e0841a3b4ef7c4331";

/// Gateway-local, negotiation-bound state for one deployment's repeated-episode
/// lease profile. Presence of this value is what may license a completed-episode
/// release to reopen admission; absence keeps the single-episode default.
///
/// The binding records the exact boot authority and instance incarnation that
/// negotiated the profile, so a later boot cannot inherit the completed-episode
/// floor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EpisodeProfile {
    capability: &'static str,
    boot_id: String,
    instance_incarnation: String,
    authority_generation: u64,
    /// Epoch of the most recently completed episode, if any. A fresh allocation
    /// of the same boot must strictly exceed this value.
    released_epoch: Option<u64>,
}

impl EpisodeProfile {
    /// Negotiates the profile from a request header. `Ok(None)` means the
    /// profile was not requested and the legacy default is unchanged;
    /// `Ok(Some(_))` binds the profile to `boot`; `Err(())` means the caller
    /// requested an unsupported profile and the request must fail closed
    /// without mutating admission state.
    pub(super) fn negotiate(
        header: Option<&str>,
        boot: &RecoveryBootContext,
    ) -> Result<Option<Self>, ()> {
        match header {
            None => Ok(None),
            Some(value) if value == EPISODE_PROFILE_NAME => Ok(Some(Self {
                capability: EPISODE_PROFILE_CAPABILITY,
                boot_id: boot.boot_id.clone(),
                instance_incarnation: boot.instance_incarnation.clone(),
                authority_generation: boot.authority_generation,
                released_epoch: None,
            })),
            Some(_) => Err(()),
        }
    }

    /// Whether this profile was negotiated against the supplied boot authority.
    pub(super) fn matches_boot(&self, boot: &RecoveryBootContext) -> bool {
        self.boot_id == boot.boot_id
            && self.instance_incarnation == boot.instance_incarnation
            && self.authority_generation == boot.authority_generation
    }

    /// Records the epoch whose episode completed successfully. The next
    /// allocation of the same boot must strictly exceed it.
    pub(super) fn record_completed(&mut self, lease_epoch: u64) {
        self.released_epoch = Some(match self.released_epoch {
            Some(previous) => previous.max(lease_epoch),
            None => lease_epoch,
        });
    }

    /// A fresh allocation is admissible only for an epoch strictly above every
    /// epoch already completed under this profile.
    pub(super) fn admits_epoch(&self, lease_epoch: u64) -> bool {
        self.released_epoch
            .is_none_or(|released| lease_epoch > released)
    }

    /// Machine-readable witness for the negotiated capability. It describes the
    /// gateway-local profile only and never carries caller secrets or host
    /// material.
    pub(super) fn witness(&self) -> Value {
        json!({
            "profile": EPISODE_PROFILE_NAME,
            "capability": self.capability,
            "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST,
            "scope": "deployment",
            "admission": "explicit-header-negotiated",
            "reopen": "completed-episode-release",
            "version": 1,
            "released_epoch": self.released_epoch,
        })
    }
}

impl RuntimeService {
    /// Negotiates an optional repeated-episode profile from release headers.
    ///
    /// A profiled release requires an available boot authority to bind against;
    /// a caller that asks for the profile when no boot exists fails closed
    /// instead of silently degrading to the single-episode default. An
    /// unsupported value is rejected before any durable write.
    pub(super) fn negotiate_episode_profile(
        &self,
        header: Option<&str>,
    ) -> Result<Option<EpisodeProfile>, (u16, Vec<u8>)> {
        let Some(boot) = self.recovery_boot.as_ref() else {
            return match header {
                None => Ok(None),
                Some(_) => Err((503, json_error("episode_profile_boot_required"))),
            };
        };
        EpisodeProfile::negotiate(header, boot)
            .map_err(|()| (400, json_error("episode_profile_unsupported")))
    }

    /// Whether a fresh allocation of `lease_epoch` must be refused because the
    /// negotiated profile already completed that epoch.
    ///
    /// A profile bound to a boot authority that has since rotated does not
    /// refuse: a new boot is a fresh admission context (bootstrap already
    /// resets the permanent stop flag), and durable lease epochs are globally
    /// monotonic, so the new boot cannot reuse an episode epoch.
    pub(super) fn episode_admission_refused(
        &self,
        boot: &RecoveryBootContext,
        lease_epoch: u64,
    ) -> bool {
        let Some(profile) = self.episode_profile.as_ref() else {
            return false;
        };
        profile.matches_boot(boot) && !profile.admits_epoch(lease_epoch)
    }

    /// Refuses a fresh allocation that would reuse or precede a completed
    /// episode.
    ///
    /// This is the fail-closed reconciliation path: the durable allocator
    /// should already have issued a strictly higher epoch for the same boot, so
    /// reaching here means the durable and local views disagree. The acquired
    /// lease is durably revoked and local admission stays closed.
    pub(super) fn refuse_released_epoch(&mut self, lease: &RecoveryLease) -> (u16, Vec<u8>) {
        if let Some(store) = self.recovery.as_mut() {
            let _ = store.revoke_lease(&lease.proof(), "shutdown");
        }
        self.lease_active = false;
        self.recovery_lease = None;
        self.recovery_lease_deadline = None;
        self.recovery_lease_deadline_lease_id = None;
        self.recovery_host_grant = None;
        self.lease_revoked = true;
        (409, json_error("lease_context_revoked"))
    }

    /// Reopens admission for the next episode after a completed-episode
    /// release.
    ///
    /// The caller has already durably revoked the completed episode's lease and
    /// received the host's revoke acknowledgment. This records the completed
    /// epoch floor, clears the permanent stop flag, drops the released lease
    /// identity, and invalidates the released episode's observed catalog so the
    /// next episode cannot consume it.
    ///
    /// A profile that no longer binds the current boot authority keeps the
    /// permanent stop flag set rather than reopening admission.
    pub(super) fn commit_completed_episode(&mut self, released_epoch: u64) {
        let bound = match (self.recovery_boot.as_ref(), self.episode_profile.as_ref()) {
            (Some(boot), Some(profile)) => profile.matches_boot(boot),
            _ => false,
        };
        if !bound {
            self.lease_revoked = true;
            return;
        }
        if let Some(profile) = self.episode_profile.as_mut() {
            profile.record_completed(released_epoch);
        }
        self.lease_revoked = false;
        // A completed boundary must not let the next episode reuse this
        // episode's observed legal-action catalog or recovery witnesses.
        self.invalidate_recovery_catalog();
        self.recovery_lease = None;
        self.recovery_lease_deadline = None;
        self.recovery_lease_deadline_lease_id = None;
        self.recovery_host_grant = None;
    }

    /// The negotiated profile witness for a response body, if any.
    pub(super) fn episode_profile_witness(&self) -> Option<Value> {
        self.episode_profile.as_ref().map(EpisodeProfile::witness)
    }
}

#[cfg(test)]
#[path = "service_episode_profile_tests.rs"]
mod tests;
