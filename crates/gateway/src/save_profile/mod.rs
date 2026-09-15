// SPDX-License-Identifier: MIT

//! Gateway-owned save-profile transport and isolated user-data provisioning.
//!
//! The game-mod remains the authority for save-slot meaning and host-thread selection.
//! This module owns only opaque identity, launch-profile binding, bounded fencing, durable
//! operation intent and the fixed forwarding seam.  It deliberately contains no host paths,
//! shell commands, URLs supplied by callers, or save contents.

mod durable_store;
mod guidance;
mod launch_profile;
mod ledger;
mod ledger_response;
#[cfg(test)]
#[path = "ledger_tests.rs"]
mod ledger_tests;
mod ledger_types;
mod ledger_validation;
mod operation_record;
mod operation_schema;
mod operation_store;
mod provisioning;
#[cfg(test)]
#[path = "provisioning_tests.rs"]
mod provisioning_tests;
mod provisioning_types;
mod provisioning_validation;
mod route;
mod types;

pub use durable_store::SqliteUserDataRecordStore;
pub use guidance::RecoveryGuidance;
pub use launch_profile::InMemoryLaunchProfileBindingPort;
pub use ledger::SaveProfileLedger;
pub use ledger_types::{
    InMemorySaveProfileRecordStore, SaveProfileForwardRequest, SaveProfileForwardResponse,
    SaveProfileForwardingPort, SaveProfileLedgerError, SaveProfileOperation,
    SaveProfileOperationRecord, SaveProfileRecordStore, SaveProfileResult, SaveProfileStatus,
    SaveProfileTransportFault,
};
pub use operation_store::{SAVE_PROFILE_OPERATION_CAPACITY, SqliteSaveProfileOperationStore};
pub use provisioning::UserDataProvisioner;
pub use provisioning_types::{
    InMemoryUserDataPort, InMemoryUserDataRecordStore, UserDataCreateOutcome,
    UserDataCreateRequest, UserDataInspection, UserDataPort, UserDataPortError,
    UserDataProvisioningError, UserDataProvisioningOutcome, UserDataProvisioningRecord,
    UserDataProvisioningStatus, UserDataRecordStore,
};
pub use route::SaveProfileRoute;
pub use types::{
    LAUNCH_PROFILE_CONTRACT, LAUNCH_PROFILE_ID, LaunchProfileBinding, LaunchProfileBindingError,
    LaunchProfileBindingPort, ProfileBaseline, SAVE_PROFILE_CONTRACT, SAVE_PROFILE_MAX_BODY_BYTES,
    SAVE_PROFILE_MAX_IDENTITY_BYTES, SAVE_PROFILE_MAX_OPERATION_BYTES, SaveProfileAuthority,
    SaveProfileContext, SaveProfileFenceError, SaveProfileId, SaveProfileIdError,
    UserDataDescriptor, UserDataIdentity, UserDataProvenance,
};
