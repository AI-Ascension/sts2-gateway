// SPDX-License-Identifier: MIT

//! Blanket forwarding for boxed and shared port handles.
//!
//! Ports are injected at composition time, so the attached runtime must be able to erase the
//! concrete adapter behind `Box<dyn Trait>` or share one clock behind `Arc`. These impls are
//! kept beside the port traits they forward, and each one forwards verbatim: a boxed port is
//! exactly as strict as the adapter it wraps because no impl adds, relaxes, or reorders a
//! check.

use std::sync::Arc;

use crate::identity::{FenceFailure, InstanceId, Lease, LeaseProof, Tick};
use crate::ports::{
    Clock, LaunchSpec, LeaseDecisionPort, ProcessFault, ProcessHandle, ProcessPort, ProcessState,
    StopMode,
};
use crate::process_identity::{ProcessDescendantIdentity, ProcessIdentity, ProcessLaunch};
use crate::process_profile::LaunchProfile;

// A boxed port keeps the attached runtime able to select its concrete adapter at composition
// time. Each method forwards to the inner implementation without adding policy, so a boxed port
// is exactly as strict as the adapter it wraps.

impl<T: ProcessPort + ?Sized> ProcessPort for Box<T> {
    fn start(&mut self, specification: LaunchSpec) -> Result<ProcessHandle, ProcessFault> {
        (**self).start(specification)
    }

    fn inspect(&mut self, process: ProcessHandle) -> Result<ProcessState, ProcessFault> {
        (**self).inspect(process)
    }

    fn stop(&mut self, process: ProcessHandle, mode: StopMode) -> Result<(), ProcessFault> {
        (**self).stop(process, mode)
    }

    fn start_with_profile(
        &mut self,
        specification: LaunchSpec,
        profile: LaunchProfile,
    ) -> Result<ProcessLaunch, ProcessFault> {
        (**self).start_with_profile(specification, profile)
    }

    fn inspect_identity(
        &mut self,
        process: ProcessHandle,
    ) -> Result<ProcessIdentity, ProcessFault> {
        (**self).inspect_identity(process)
    }

    fn descendants(
        &mut self,
        process: ProcessHandle,
    ) -> Result<Vec<ProcessDescendantIdentity>, ProcessFault> {
        (**self).descendants(process)
    }

    fn recover_owned(
        &mut self,
        instance_id: InstanceId,
        profile: LaunchProfile,
    ) -> Result<Option<ProcessIdentity>, ProcessFault> {
        (**self).recover_owned(instance_id, profile)
    }
}

impl<T: Clock + ?Sized> Clock for Box<T> {
    fn now(&self) -> Tick {
        (**self).now()
    }
}

// A shared clock handle lets the attached runtime read the exact same monotonic origin the
// coordinator fences against. Two independent clock instances would disagree about expiry and
// reject valid requests, so the adapter must be able to hand out one shared instance.
impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> Tick {
        (**self).now()
    }
}

impl<T: LeaseDecisionPort + ?Sized> LeaseDecisionPort for Box<T> {
    fn check_fence(
        &mut self,
        current: Option<Lease>,
        target: InstanceId,
        proof: LeaseProof,
        now: Tick,
    ) -> Result<(), FenceFailure> {
        (**self).check_fence(current, target, proof, now)
    }
}
