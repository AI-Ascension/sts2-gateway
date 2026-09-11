// SPDX-License-Identifier: MIT

/// Fixed additive expert-state routes. The gateway owns method/path admission while the mod
/// remains authoritative for observation and potion mutation semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertRoute {
    State,
    Dispatch,
    Reconcile(String),
}

impl RuntimeV4ExpertRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v4/instances/{instance_id}/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("GET", "expert-state") => Some(Self::State),
            ("POST", "expert-action") => Some(Self::Dispatch),
            ("GET", operation_path) => {
                let operation_id = operation_path.strip_prefix("expert-actions/")?;
                safe_operation_id(operation_id).then(|| Self::Reconcile(operation_id.to_owned()))
            }
            _ => None,
        }
    }

    pub(crate) fn downstream_path(&self) -> String {
        match self {
            Self::State => String::from("/api/v4/runtime/expert-state"),
            Self::Dispatch => String::from("/api/v4/runtime/expert-action"),
            Self::Reconcile(operation_id) => {
                format!("/api/v4/runtime/expert-actions/{operation_id}")
            }
        }
    }

    pub(crate) const fn is_state(&self) -> bool {
        matches!(self, Self::State)
    }

    pub(crate) const fn is_dispatch(&self) -> bool {
        matches!(self, Self::Dispatch)
    }

    pub(crate) fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Reconcile(operation_id) => Some(operation_id),
            _ => None,
        }
    }
}

fn safe_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && !value.contains('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
