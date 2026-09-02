// SPDX-License-Identifier: MIT

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Warning => formatter.write_str("WARNING"),
            Self::Error => formatter.write_str("ERROR"),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Finding {
    pub(crate) severity: Severity,
    pub(crate) rule: &'static str,
    pub(crate) path: String,
    pub(crate) message: String,
}

impl Finding {
    pub(crate) fn warning(rule: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, rule, path, message)
    }

    pub(crate) fn error(rule: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, rule, path, message)
    }

    fn new(severity: Severity, rule: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self {
            severity,
            rule,
            path: path.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn render(&self) -> String {
        format!(
            "{} {} {}: {}",
            self.severity, self.rule, self.path, self.message
        )
    }
}
