// SPDX-License-Identifier: MIT

/// Gateway-owned fixed save-profile routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileRoute {
    List,
    Current,
    Select,
    CreateDisposable,
    Lookup,
}

impl SaveProfileRoute {
    pub const fn is_mutation(self) -> bool {
        matches!(self, Self::Select | Self::CreateDisposable)
    }

    pub const fn requires_body(self) -> bool {
        matches!(self, Self::Select | Self::CreateDisposable)
    }

    pub const fn downstream_path(self) -> &'static str {
        match self {
            Self::List => "/api/v1/save-profiles",
            Self::Current => "/api/v1/save-profile/current",
            Self::Select => "/api/v1/save-profile/select",
            Self::CreateDisposable => "/api/v1/save-profile/create-disposable",
            Self::Lookup => "/api/v1/save-profile/operations",
        }
    }
}
