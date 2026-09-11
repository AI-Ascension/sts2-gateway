// SPDX-License-Identifier: MIT

/// The additive, read-only map route.  Route ownership stays in the gateway;
/// map meaning and projection remain downstream host/protocol concerns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeMapRoute {
    Snapshot,
}

impl RuntimeMapRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let expected = format!("/v1/instances/{instance_id}/map-snapshot");
        (method == "GET" && path == expected).then_some(Self::Snapshot)
    }

    pub(crate) const fn downstream_path(self) -> &'static str {
        match self {
            Self::Snapshot => "/api/map/v1/snapshot",
        }
    }
}
