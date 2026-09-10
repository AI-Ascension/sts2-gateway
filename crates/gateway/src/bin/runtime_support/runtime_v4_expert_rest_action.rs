// SPDX-License-Identifier: MIT

/// Fixed routes for the candidate native rest-site action profile. The gateway owns only
/// method/path admission; the game-mod remains authoritative for option and selector meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV4ExpertRestActionRoute {
    Dispatch,
    Reconcile(String),
}

impl RuntimeV4ExpertRestActionRoute {
    pub(crate) fn parse(method: &str, path: &str, instance_id: &str) -> Option<Self> {
        let prefix = format!("/v4/instances/{instance_id}/");
        let suffix = path.strip_prefix(&prefix)?;
        match (method, suffix) {
            ("POST", "expert-rest-action") => Some(Self::Dispatch),
            ("GET", operation_path) => {
                let operation_id = operation_path.strip_prefix("expert-rest-actions/")?;
                safe_operation_id(operation_id).then(|| Self::Reconcile(operation_id.to_owned()))
            }
            _ => None,
        }
    }

    pub(crate) fn downstream_path(&self) -> String {
        match self {
            Self::Dispatch => String::from("/api/v4/runtime/expert-rest-action"),
            Self::Reconcile(operation_id) => {
                format!("/api/v4/runtime/expert-rest-actions/{operation_id}")
            }
        }
    }

    pub(crate) const fn is_dispatch(&self) -> bool {
        matches!(self, Self::Dispatch)
    }

    pub(crate) fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Dispatch => None,
            Self::Reconcile(operation_id) => Some(operation_id),
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
