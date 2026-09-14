// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::*;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub(super) fn context(correlation: &str) -> SaveProfileContext {
    SaveProfileContext {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        correlation_id: correlation.to_owned(),
    }
}

fn authority() -> SaveProfileAuthority {
    SaveProfileAuthority {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        expires_at_millis: None,
    }
}

fn baseline() -> ProfileBaseline {
    ProfileBaseline {
        identity: String::from("baseline-1"),
        digest: String::from(DIGEST),
    }
}

fn descriptor(identity: u64, operation_id: &str) -> UserDataDescriptor {
    UserDataDescriptor {
        identity: UserDataIdentity::new(identity),
        provenance: UserDataProvenance {
            owner: String::from("gateway"),
            instance_id: String::from("instance-1"),
            operation_id: operation_id.to_owned(),
            contract: String::from(LAUNCH_PROFILE_CONTRACT),
        },
        baseline: None,
    }
}

#[derive(Default)]
pub(super) struct FakeForwarder {
    pub(super) responses: VecDeque<Result<SaveProfileForwardResponse, SaveProfileTransportFault>>,
    pub(super) forwards: usize,
}

impl SaveProfileForwardingPort for FakeForwarder {
    fn forward(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
        self.forwards += 1;
        self.responses
            .pop_front()
            .unwrap_or(Err(SaveProfileTransportFault::UnavailableBeforeWrite))
    }

    fn lookup(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<Option<SaveProfileForwardResponse>, SaveProfileTransportFault> {
        Ok(None)
    }
}

pub(super) fn create_request(operation_id: &str) -> Result<SaveProfileForwardRequest, String> {
    let user_data = descriptor(1, operation_id);
    let launch_profile = LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, user_data.identity)
        .map_err(|_| String::from("static launch profile is invalid"))?;
    Ok(SaveProfileForwardRequest {
        operation_id: operation_id.to_owned(),
        context: context("corr-create"),
        route: SaveProfileRoute::CreateDisposable,
        operation: SaveProfileOperation::CreateDisposable {
            launch_profile,
            user_data,
        },
        body: b"{}".to_vec(),
    })
}

pub(super) fn select_request(operation_id: &str) -> Result<SaveProfileForwardRequest, String> {
    Ok(SaveProfileForwardRequest {
        operation_id: operation_id.to_owned(),
        context: context("corr-select"),
        route: SaveProfileRoute::Select,
        operation: SaveProfileOperation::Select {
            profile_id: SaveProfileId::try_new("slot-1").map_err(|_| String::from("id"))?,
            baseline: baseline(),
        },
        body: b"{\"profile_id\":\"slot-1\"}".to_vec(),
    })
}

pub(super) fn settled(
    request: &SaveProfileForwardRequest,
    profile_id: Option<SaveProfileId>,
    user_data: Option<UserDataDescriptor>,
) -> SaveProfileForwardResponse {
    SaveProfileForwardResponse {
        operation_id: request.operation_id.clone(),
        context: request.context.clone(),
        route: request.route,
        status: SaveProfileStatus::Settled,
        body: b"{\"status\":\"settled\"}".to_vec(),
        profile_id,
        baseline: Some(baseline()),
        user_data,
        reason: None,
    }
}

/// Shared operation-intent store. A clone reopens the same records, which is how the
/// deterministic tests model a gateway restart over a durable store.
#[derive(Clone, Default)]
pub(super) struct SharedIntentStore {
    records: Arc<Mutex<BTreeMap<(String, String), SaveProfileOperationRecord>>>,
}

impl SaveProfileRecordStore for SharedIntentStore {
    fn list(&mut self) -> Result<Vec<SaveProfileOperationRecord>, SaveProfileLedgerError> {
        let records = self
            .records
            .lock()
            .map_err(|_| SaveProfileLedgerError::PersistenceFailed)?;
        Ok(records.values().cloned().collect())
    }

    fn insert(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| SaveProfileLedgerError::PersistenceFailed)?;
        let key = (
            record.request.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if records.contains_key(&key) {
            return Err(SaveProfileLedgerError::OperationConflict);
        }
        records.insert(key, record);
        Ok(())
    }

    fn update(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| SaveProfileLedgerError::PersistenceFailed)?;
        let key = (
            record.request.context.instance_id.clone(),
            record.operation_id.clone(),
        );
        if !records.contains_key(&key) {
            return Err(SaveProfileLedgerError::OperationNotFound);
        }
        records.insert(key, record);
        Ok(())
    }
}

fn open_ledger<F: SaveProfileForwardingPort, S: SaveProfileRecordStore>(
    forwarding: F,
    store: S,
) -> Result<SaveProfileLedger<F, S>, String> {
    SaveProfileLedger::new(8, authority(), forwarding, store).map_err(|error| format!("{error:?}"))
}

#[test]
fn creation_receipt_must_echo_the_reserved_identity_and_launch_contract() -> Result<(), String> {
    let request = create_request("create-1")?;
    let mut foreign = descriptor(1, "create-1");
    foreign.provenance.contract = String::from("foreign-launch-contract");
    for receipt in [descriptor(9, "create-1"), foreign] {
        let mut fake = FakeForwarder::default();
        fake.responses
            .push_back(Ok(settled(&request, None, Some(receipt))));
        let mut ledger = open_ledger(fake, InMemorySaveProfileRecordStore::default())?;
        assert_eq!(
            ledger.submit(request.clone(), 0, false),
            Err(SaveProfileLedgerError::ResponseInvalid)
        );
        assert_eq!(ledger.forwarding_mut().forwards, 1);
    }

    let mut fake = FakeForwarder::default();
    fake.responses
        .push_back(Ok(settled(&request, None, Some(descriptor(1, "create-1")))));
    let mut ledger = open_ledger(fake, InMemorySaveProfileRecordStore::default())?;
    let result = ledger
        .submit(request, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result.status, SaveProfileStatus::Settled);
    assert_eq!(result.user_data, Some(descriptor(1, "create-1")));
    Ok(())
}

#[test]
fn duplicate_selection_with_a_distinct_operation_is_refused() -> Result<(), String> {
    let first = select_request("select-1")?;
    let mut fake = FakeForwarder::default();
    let slot = SaveProfileId::try_new("slot-1").map_err(|_| String::from("id"))?;
    fake.responses
        .push_back(Ok(settled(&first, Some(slot), None)));
    let mut ledger = open_ledger(fake, InMemorySaveProfileRecordStore::default())?;
    ledger
        .submit(first, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        ledger.submit(select_request("select-2")?, 0, false),
        Err(SaveProfileLedgerError::DuplicateSelection)
    );
    assert_eq!(ledger.forwarding_mut().forwards, 1);
    Ok(())
}

#[test]
fn reopened_intent_store_never_dispatches_a_retained_mutation_again() -> Result<(), String> {
    let store = SharedIntentStore::default();
    let mut first = FakeForwarder::default();
    first
        .responses
        .push_back(Err(SaveProfileTransportFault::TimeoutAfterWrite));
    let mut ledger = open_ledger(first, store.clone())?;
    let unknown = ledger
        .submit(select_request("select-1")?, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(unknown.status, SaveProfileStatus::Unknown);

    let mut reopened = open_ledger(FakeForwarder::default(), store)?;
    let replayed = reopened
        .submit(select_request("select-1")?, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(replayed.status, SaveProfileStatus::Unknown);
    assert_eq!(reopened.forwarding_mut().forwards, 0);
    assert_eq!(
        reopened.submit(select_request("select-2")?, 0, false),
        Err(SaveProfileLedgerError::DuplicateSelection)
    );
    assert_eq!(reopened.forwarding_mut().forwards, 0);
    Ok(())
}
