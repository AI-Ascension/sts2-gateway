// SPDX-License-Identifier: MIT

//! Fixed gateway routes for the accepted `coop-native-v1` component contract.
//!
//! The gateway owns the instance, lease, and transport fences.  The game-mod remains the only
//! owner of native peer identity, game legality, and effect settlement.  This module therefore
//! validates the closed protocol envelope and forwards only the six fixed downstream paths.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoopNativeRoute {
    Observation,
    LegalCatalog,
    LocalAction,
    SharedVote,
    Rejoin,
    Recover,
}

impl CoopNativeRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v1/instances/{instance_id}/coop/native/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("GET", "observation") => Some(Self::Observation),
            ("POST", "legal-catalog") => Some(Self::LegalCatalog),
            ("POST", "action") => Some(Self::LocalAction),
            ("POST", "vote") => Some(Self::SharedVote),
            ("POST", "rejoin") => Some(Self::Rejoin),
            ("POST", "recover") => Some(Self::Recover),
            _ => None,
        }
    }

    pub(crate) const fn downstream_path(self) -> &'static str {
        match self {
            Self::Observation => "/api/v1/coop/native/observation",
            Self::LegalCatalog => "/api/v1/coop/native/legal-catalog",
            Self::LocalAction => "/api/v1/coop/native/action",
            Self::SharedVote => "/api/v1/coop/native/vote",
            Self::Rejoin => "/api/v1/coop/native/rejoin",
            Self::Recover => "/api/v1/coop/native/recover",
        }
    }

    pub(crate) const fn request_kind(self) -> Option<&'static str> {
        match self {
            Self::Observation => None,
            Self::LegalCatalog => Some("legal_catalog_request"),
            Self::LocalAction => Some("local_action_request"),
            Self::SharedVote => Some("shared_vote_request"),
            Self::Rejoin => Some("rejoin_request"),
            Self::Recover => Some("recovery_response"),
        }
    }

    pub(crate) const fn response_kind(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::LegalCatalog => "legal_catalog_response",
            Self::LocalAction | Self::SharedVote => "effect_response",
            Self::Rejoin | Self::Recover => "recovery_response",
        }
    }

    pub(crate) const fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::LocalAction | Self::SharedVote | Self::Rejoin | Self::Recover
        )
    }

    pub(crate) const fn is_control(self) -> bool {
        matches!(self, Self::Rejoin | Self::Recover)
    }
}
