// SPDX-License-Identifier: MIT

use sts2_gateway::SaveProfileRoute;

/// Fixed save-profile routes owned by the gateway.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeSaveProfileRoute {
    List,
    Current,
    Select,
    CreateDisposable,
    Lookup,
}

impl RuntimeSaveProfileRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v1/instances/{instance_id}/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("GET", "save-profiles") => Some(Self::List),
            ("GET", "save-profile/current") => Some(Self::Current),
            ("POST", "save-profile/select") => Some(Self::Select),
            ("POST", "save-profile/create-disposable") => Some(Self::CreateDisposable),
            ("GET", suffix) => suffix
                .strip_prefix("save-profile/operations/")
                .filter(|id| safe_operation_id(id))
                .map(|_| Self::Lookup),
            _ => None,
        }
    }

    pub(crate) const fn contract_route(self) -> SaveProfileRoute {
        match self {
            Self::List => SaveProfileRoute::List,
            Self::Current => SaveProfileRoute::Current,
            Self::Select => SaveProfileRoute::Select,
            Self::CreateDisposable => SaveProfileRoute::CreateDisposable,
            Self::Lookup => SaveProfileRoute::Lookup,
        }
    }

    pub(crate) const fn is_mutation(self) -> bool {
        self.contract_route().is_mutation()
    }

    pub(crate) fn operation_id<'a>(self, path: &'a str, instance_id: &str) -> Option<&'a str> {
        (self == Self::Lookup)
            .then(|| {
                let prefix = format!("/v1/instances/{instance_id}/save-profile/operations/");
                path.strip_prefix(&prefix)
            })
            .flatten()
            .filter(|id| safe_operation_id(id))
    }
}

fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains('/')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
