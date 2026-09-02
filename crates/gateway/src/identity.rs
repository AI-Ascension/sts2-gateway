// SPDX-License-Identifier: MIT

use std::fmt;

macro_rules! identifier {
    ($name:ident, $description:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            /// Creates an opaque identifier from its caller-owned value.
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Returns the opaque numeric representation.
            pub const fn value(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}:{}", $description, self.0)
            }
        }
    };
}

identifier!(CallerId, "caller");
identifier!(SessionId, "session");
identifier!(InstanceId, "instance");
identifier!(LeaseId, "lease");
identifier!(OperationId, "operation");

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    /// Creates a monotonic test or runtime tick measured in milliseconds.
    pub const fn from_millis(value: u64) -> Self {
        Self(value)
    }

    /// Returns the monotonic millisecond value.
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub const fn saturating_add_millis(self, duration: u64) -> Self {
        Self(self.0.saturating_add(duration))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LeaseEpoch(u64);

impl LeaseEpoch {
    /// Creates an epoch for test fixtures or a persisted control-plane record.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the epoch value used by fencing comparisons.
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lease {
    instance_id: InstanceId,
    caller_id: CallerId,
    session_id: SessionId,
    lease_id: LeaseId,
    epoch: LeaseEpoch,
    expires_at: Tick,
}

impl Lease {
    pub(crate) const fn new(
        instance_id: InstanceId,
        caller_id: CallerId,
        session_id: SessionId,
        lease_id: LeaseId,
        epoch: LeaseEpoch,
        expires_at: Tick,
    ) -> Self {
        Self {
            instance_id,
            caller_id,
            session_id,
            lease_id,
            epoch,
            expires_at,
        }
    }

    pub(crate) const fn renewed(self, expires_at: Tick) -> Self {
        Self { expires_at, ..self }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn caller_id(self) -> CallerId {
        self.caller_id
    }

    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    pub const fn lease_id(self) -> LeaseId {
        self.lease_id
    }

    pub const fn epoch(self) -> LeaseEpoch {
        self.epoch
    }

    pub const fn expires_at(self) -> Tick {
        self.expires_at
    }

    /// Produces the identity-bearing proof required by control and data operations.
    pub const fn proof(self) -> LeaseProof {
        LeaseProof {
            instance_id: self.instance_id,
            caller_id: self.caller_id,
            session_id: self.session_id,
            lease_id: self.lease_id,
            epoch: self.epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseProof {
    instance_id: InstanceId,
    caller_id: CallerId,
    session_id: SessionId,
    lease_id: LeaseId,
    epoch: LeaseEpoch,
}

impl LeaseProof {
    /// Creates a proof value for boundary tests or an authenticated adapter.
    pub const fn new(
        instance_id: InstanceId,
        caller_id: CallerId,
        session_id: SessionId,
        lease_id: LeaseId,
        epoch: LeaseEpoch,
    ) -> Self {
        Self {
            instance_id,
            caller_id,
            session_id,
            lease_id,
            epoch,
        }
    }

    pub const fn instance_id(self) -> InstanceId {
        self.instance_id
    }

    pub const fn caller_id(self) -> CallerId {
        self.caller_id
    }

    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    pub const fn lease_id(self) -> LeaseId {
        self.lease_id
    }

    pub const fn epoch(self) -> LeaseEpoch {
        self.epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FenceFailure {
    Missing,
    WrongInstance,
    WrongCaller,
    WrongSession,
    WrongLease,
    StaleEpoch,
    Expired,
}

/// Evaluates the complete lease identity and epoch fence without performing I/O.
pub fn evaluate_fence(
    current: Option<&Lease>,
    target: InstanceId,
    proof: LeaseProof,
    now: Tick,
) -> Result<(), FenceFailure> {
    if proof.instance_id != target {
        return Err(FenceFailure::WrongInstance);
    }
    let Some(current) = current else {
        return Err(FenceFailure::Missing);
    };
    if current.expires_at <= now {
        return Err(FenceFailure::Expired);
    }
    if current.caller_id != proof.caller_id {
        return Err(FenceFailure::WrongCaller);
    }
    if current.session_id != proof.session_id {
        return Err(FenceFailure::WrongSession);
    }
    if current.lease_id != proof.lease_id {
        return Err(FenceFailure::WrongLease);
    }
    if current.epoch != proof.epoch {
        return Err(FenceFailure::StaleEpoch);
    }
    Ok(())
}
