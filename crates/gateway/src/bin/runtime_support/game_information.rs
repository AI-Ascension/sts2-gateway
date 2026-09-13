// SPDX-License-Identifier: MIT

/// Fixed, read-only game-information operations.  The operation is part of
/// the route allowlist; it is never selected from a caller-supplied URL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GameInformationRoute {
    Capabilities,
    /// Canonical MCP-facing route.  The query envelope selects one of the
    /// fixed producer operations; the URL itself never becomes a downstream
    /// path.
    Query,
    List,
    Search,
    Get,
    Detail,
    Availability,
}

impl GameInformationRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v1/instances/{instance_id}/game-information/");
        let operation = path.strip_prefix(&prefix)?;
        match (method, operation) {
            ("GET", "capabilities") => Some(Self::Capabilities),
            ("POST", "query") => Some(Self::Query),
            ("POST", "list") => Some(Self::List),
            ("POST", "search") => Some(Self::Search),
            ("POST", "get") => Some(Self::Get),
            ("POST", "detail") => Some(Self::Detail),
            ("POST", "availability") => Some(Self::Availability),
            _ => None,
        }
    }

    pub(crate) const fn is_query(self) -> bool {
        !matches!(self, Self::Capabilities)
    }

    pub(crate) const fn query_kind(self) -> Option<&'static str> {
        match self {
            Self::Capabilities | Self::Query => None,
            Self::List => Some("list"),
            Self::Search => Some("search"),
            Self::Get => Some("get"),
            Self::Detail => Some("detail"),
            Self::Availability => Some("availability"),
        }
    }

    pub(crate) const fn downstream_path(self) -> &'static str {
        match self {
            Self::Capabilities => "/api/v1/game-information/capabilities",
            // Callers must resolve `Query` with `from_query_kind` before
            // forwarding.  Returning an empty path makes accidental use fail
            // closed at the HTTP transport boundary.
            Self::Query => "",
            Self::List => "/api/v1/game-information/list",
            Self::Search => "/api/v1/game-information/search",
            Self::Get => "/api/v1/game-information/get",
            Self::Detail => "/api/v1/game-information/detail",
            Self::Availability => "/api/v1/game-information/availability",
        }
    }

    pub(crate) fn from_query_kind(kind: &str) -> Option<Self> {
        match kind {
            "list" => Some(Self::List),
            "search" => Some(Self::Search),
            "get" => Some(Self::Get),
            "detail" => Some(Self::Detail),
            "availability" => Some(Self::Availability),
            _ => None,
        }
    }
}
