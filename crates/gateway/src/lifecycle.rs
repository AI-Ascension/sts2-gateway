// SPDX-License-Identifier: MIT

use crate::identity::{CallerId, InstanceId, Lease, SessionId};
use crate::ports::ProcessHandle;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    Created,
    Starting,
    Ready,
    Busy,
    Degraded,
    Stopping,
    Stopped,
    Failed,
    Expired,
}

impl LifecycleState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed | Self::Expired)
    }

    pub const fn accepts_forwarding(self) -> bool {
        matches!(self, Self::Ready | Self::Busy)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceSnapshot {
    instance_id: InstanceId,
    caller_id: CallerId,
    session_id: SessionId,
    state: LifecycleState,
    has_process: bool,
    lease: Option<Lease>,
}

impl InstanceSnapshot {
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub const fn caller_id(&self) -> CallerId {
        self.caller_id
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    pub const fn has_process(&self) -> bool {
        self.has_process
    }

    pub const fn lease(&self) -> Option<Lease> {
        self.lease
    }
}

#[derive(Debug)]
pub(crate) struct InstanceRecord {
    instance_id: InstanceId,
    caller_id: CallerId,
    session_id: SessionId,
    state: LifecycleState,
    process: Option<ProcessHandle>,
    lease: Option<Lease>,
}

impl InstanceRecord {
    pub(crate) const fn new(
        instance_id: InstanceId,
        caller_id: CallerId,
        session_id: SessionId,
        lease: Lease,
    ) -> Self {
        Self {
            instance_id,
            caller_id,
            session_id,
            state: LifecycleState::Created,
            process: None,
            lease: Some(lease),
        }
    }

    pub(crate) fn assign_process(&mut self, process: ProcessHandle) {
        self.process = Some(process);
        self.state = LifecycleState::Starting;
    }

    pub(crate) fn set_lease(&mut self, lease: Lease) {
        self.lease = Some(lease);
    }

    pub(crate) fn mark_ready(&mut self) {
        self.state = LifecycleState::Ready;
    }

    pub(crate) fn mark_starting(&mut self) {
        self.state = LifecycleState::Starting;
    }

    pub(crate) fn mark_busy(&mut self) {
        self.state = LifecycleState::Busy;
    }

    pub(crate) fn mark_degraded(&mut self) {
        self.state = LifecycleState::Degraded;
    }

    pub(crate) fn mark_stopping(&mut self) {
        self.state = LifecycleState::Stopping;
    }

    pub(crate) fn mark_stopped(&mut self) {
        self.state = LifecycleState::Stopped;
        self.lease = None;
    }

    pub(crate) fn mark_failed(&mut self) {
        self.state = LifecycleState::Failed;
        self.lease = None;
    }

    pub(crate) fn mark_expired(&mut self) {
        self.state = LifecycleState::Expired;
        self.lease = None;
    }

    pub(crate) fn clear_process(&mut self) {
        self.process = None;
    }

    pub(crate) const fn caller_id(&self) -> CallerId {
        self.caller_id
    }

    pub(crate) const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub(crate) const fn state(&self) -> LifecycleState {
        self.state
    }

    pub(crate) const fn process(&self) -> Option<ProcessHandle> {
        self.process
    }

    pub(crate) const fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    pub(crate) fn snapshot(&self) -> InstanceSnapshot {
        InstanceSnapshot {
            instance_id: self.instance_id,
            caller_id: self.caller_id,
            session_id: self.session_id,
            state: self.state,
            has_process: self.process.is_some(),
            lease: self.lease,
        }
    }
}
