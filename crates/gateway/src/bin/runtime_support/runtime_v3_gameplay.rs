// SPDX-License-Identifier: MIT

/// Fixed Runtime-v3 gameplay routes. The gateway owns the route and method
/// allowlist; the game-mod owns the meaning of each request and response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV3GameplayRoute {
    State,
    LegalActions,
    DispatchAction,
    WaitForTransition,
    Reobserve,
    Recover,
}

impl RuntimeV3GameplayRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v3/instances/{instance_id}/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("GET", "state") => Some(Self::State),
            ("GET", "legal-actions") => Some(Self::LegalActions),
            ("POST", "action") => Some(Self::DispatchAction),
            ("POST", "wait") => Some(Self::WaitForTransition),
            ("GET", "reobserve") => Some(Self::Reobserve),
            ("POST", "recover") => Some(Self::Recover),
            _ => None,
        }
    }

    pub(crate) const fn downstream_path(self) -> &'static str {
        match self {
            Self::State => "/api/v3/runtime/state",
            Self::LegalActions => "/api/v3/runtime/legal-actions",
            Self::DispatchAction => "/api/v3/runtime/action",
            Self::WaitForTransition => "/api/v3/runtime/wait",
            Self::Reobserve => "/api/v3/runtime/reobserve",
            Self::Recover => "/api/v3/runtime/recover",
        }
    }

    pub(crate) const fn is_post(self) -> bool {
        matches!(
            self,
            Self::DispatchAction | Self::WaitForTransition | Self::Recover
        )
    }

    pub(crate) const fn request_kind(self) -> &'static str {
        match self {
            Self::State => "state_request",
            Self::LegalActions => "legal_actions_request",
            Self::DispatchAction => "dispatch_action_request",
            Self::WaitForTransition => "wait_request",
            Self::Reobserve => "reobserve_request",
            Self::Recover => "recover_request",
        }
    }

    pub(crate) const fn response_kind(self) -> &'static str {
        match self {
            Self::State => "state_response",
            Self::LegalActions => "legal_actions_response",
            Self::DispatchAction => "dispatch_action_response",
            Self::WaitForTransition => "wait_response",
            Self::Reobserve => "reobserve_response",
            Self::Recover => "recover_response",
        }
    }
}
