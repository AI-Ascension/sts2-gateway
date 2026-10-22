// SPDX-License-Identifier: MIT

use crate::{FenceFailure, HealthFault, LifecycleState, ProcessFault, TransportFault};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayError {
    AdmissionClosed,
    CapacityExceeded,
    IdentityExhausted,
    InstanceNotFound,
    IdentityMismatch,
    InvalidState(LifecycleState),
    Fence(FenceFailure),
    ProcessStart(ProcessFault),
    ProcessInspection(ProcessFault),
    ProcessCrashed,
    ProcessStop(ProcessFault),
    Readiness(HealthFault),
    BodyTooLarge { limit: usize, actual: usize },
    ResponseTooLarge { limit: usize, actual: usize },
    Transport(TransportFault),
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdmissionClosed => formatter.write_str("gateway admission is closed"),
            Self::CapacityExceeded => formatter.write_str("gateway capacity is exhausted"),
            Self::IdentityExhausted => formatter.write_str("gateway identity space is exhausted"),
            Self::InstanceNotFound => formatter.write_str("instance was not found"),
            Self::IdentityMismatch => formatter.write_str("caller or session identity mismatched"),
            Self::InvalidState(state) => {
                write!(formatter, "operation is invalid in state {state:?}")
            }
            Self::Fence(failure) => write!(formatter, "lease fence rejected request: {failure:?}"),
            Self::ProcessStart(fault) => write!(formatter, "process start failed: {fault:?}"),
            Self::ProcessInspection(fault) => {
                write!(formatter, "process inspection failed: {fault:?}")
            }
            Self::ProcessCrashed => formatter.write_str("owned process exited unexpectedly"),
            Self::ProcessStop(fault) => write!(formatter, "process stop failed: {fault:?}"),
            Self::Readiness(fault) => write!(formatter, "readiness probe failed: {fault:?}"),
            Self::BodyTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "request body has {actual} bytes; limit is {limit}"
                )
            }
            Self::ResponseTooLarge { limit, actual } => {
                write!(
                    formatter,
                    "response body has {actual} bytes; limit is {limit}"
                )
            }
            Self::Transport(fault) => write!(formatter, "downstream transport failed: {fault:?}"),
        }
    }
}

impl std::error::Error for GatewayError {}
