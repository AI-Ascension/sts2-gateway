// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::guidance::RecoveryGuidance;
use super::route::SaveProfileRoute;
use super::types::{
    LaunchProfileBinding, ProfileBaseline, SaveProfileContext, SaveProfileId, UserDataDescriptor,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveProfileOperation {
    List,
    Current,
    Select {
        profile_id: SaveProfileId,
        baseline: ProfileBaseline,
    },
    CreateDisposable {
        launch_profile: LaunchProfileBinding,
        user_data: UserDataDescriptor,
    },
}

impl SaveProfileOperation {
    pub const fn route(&self) -> SaveProfileRoute {
        match self {
            Self::List => SaveProfileRoute::List,
            Self::Current => SaveProfileRoute::Current,
            Self::Select { .. } => SaveProfileRoute::Select,
            Self::CreateDisposable { .. } => SaveProfileRoute::CreateDisposable,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileForwardRequest {
    pub operation_id: String,
    pub context: SaveProfileContext,
    pub route: SaveProfileRoute,
    pub operation: SaveProfileOperation,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileForwardResponse {
    pub operation_id: String,
    pub context: SaveProfileContext,
    pub route: SaveProfileRoute,
    pub status: SaveProfileStatus,
    pub body: Vec<u8>,
    pub profile_id: Option<SaveProfileId>,
    pub baseline: Option<ProfileBaseline>,
    pub user_data: Option<UserDataDescriptor>,
    pub reason: Option<String>,
}

impl SaveProfileForwardResponse {
    pub fn rejected(
        request: &SaveProfileForwardRequest,
        status: SaveProfileStatus,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            operation_id: request.operation_id.clone(),
            context: request.context.clone(),
            route: request.route,
            status,
            body: Vec::new(),
            profile_id: None,
            baseline: None,
            user_data: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileStatus {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Blocked,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileTransportFault {
    UnavailableBeforeWrite,
    RejectedBeforeWrite,
    TimeoutAfterWrite,
    DisconnectedAfterWrite,
    MalformedResponse,
}

pub trait SaveProfileForwardingPort {
    fn forward(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault>;

    fn lookup(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<Option<SaveProfileForwardResponse>, SaveProfileTransportFault>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileResult {
    pub operation_id: String,
    pub route: SaveProfileRoute,
    pub status: SaveProfileStatus,
    pub body: Vec<u8>,
    pub profile_id: Option<SaveProfileId>,
    pub baseline: Option<ProfileBaseline>,
    pub user_data: Option<UserDataDescriptor>,
    pub guidance: Option<RecoveryGuidance>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveProfileOperationRecord {
    pub operation_id: String,
    pub request: SaveProfileForwardRequest,
    pub status: SaveProfileStatus,
    pub result: Option<SaveProfileResult>,
}

pub trait SaveProfileRecordStore {
    fn list(&mut self) -> Result<Vec<SaveProfileOperationRecord>, SaveProfileLedgerError>;
    fn insert(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError>;
    fn update(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError>;
}

#[derive(Clone, Debug, Default)]
pub struct InMemorySaveProfileRecordStore {
    records: BTreeMap<(String, String), SaveProfileOperationRecord>,
}

impl SaveProfileRecordStore for InMemorySaveProfileRecordStore {
    fn list(&mut self) -> Result<Vec<SaveProfileOperationRecord>, SaveProfileLedgerError> {
        Ok(self.records.values().cloned().collect())
    }

    fn insert(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        let key = (
            record.request.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if self.records.contains_key(&key) {
            return Err(SaveProfileLedgerError::OperationConflict);
        }
        self.records.insert(key, record);
        Ok(())
    }

    fn update(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        let key = (
            record.request.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if !self.records.contains_key(&key) {
            return Err(SaveProfileLedgerError::OperationNotFound);
        }
        self.records.insert(key, record);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SaveProfileLedgerError {
    InvalidRequest,
    Fence(super::types::SaveProfileFenceError),
    ActiveRun,
    BaselineRequired,
    OperationConflict,
    OperationNotFound,
    OperationInProgress,
    DuplicateSelection,
    CapacityExceeded,
    PersistenceFailed,
    Transport(SaveProfileTransportFault),
    ResponseInvalid,
}
