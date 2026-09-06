// SPDX-License-Identifier: MIT

/// The additive expert-state route returns one serialized host-owned observation. It has no
/// action side effect; the generation and legal-action identities are consumed by downstream
/// provider code and must be refreshed before any mutation is attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertRoute {
    State,
}

impl RuntimeV4ExpertRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let expected = format!("/v4/instances/{instance_id}/expert-state");
        (method == "GET" && path == expected).then_some(Self::State)
    }

    pub(crate) const fn downstream_path(self) -> &'static str {
        match self {
            Self::State => "/api/v4/runtime/expert-state",
        }
    }
}
