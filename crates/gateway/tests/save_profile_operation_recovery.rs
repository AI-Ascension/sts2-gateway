// SPDX-License-Identifier: MIT

// Fixture setup failures must fail the test; production error handling retains denied unwrap/expect.
#![allow(clippy::expect_used)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use sts2_gateway::*;

#[path = "save_profile_operation_recovery/storage_faults.rs"]
mod storage_faults;

struct Journal(PathBuf);
impl Journal {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        std::fs::create_dir_all(&root).expect("rebuildable test root");
        let path = root.join(format!("gateway-operation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("test directory");
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("operations.sqlite")
    }
    fn open(&self) -> SqliteSaveProfileOperationStore {
        SqliteSaveProfileOperationStore::open(self.path()).expect("test store")
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context() -> SaveProfileContext {
    SaveProfileContext {
        instance_id: "instance".into(),
        caller_id: "caller".into(),
        session_id: "session".into(),
        lease_id: "lease".into(),
        lease_epoch: 1,
        correlation_id: "correlation".into(),
    }
}
fn authority() -> SaveProfileAuthority {
    SaveProfileAuthority {
        instance_id: "instance".into(),
        caller_id: "caller".into(),
        session_id: "session".into(),
        lease_id: "lease".into(),
        lease_epoch: 1,
        expires_at_millis: None,
    }
}
fn baseline() -> ProfileBaseline {
    ProfileBaseline {
        identity: "baseline".into(),
        digest: "a".repeat(64),
    }
}
fn select(id: &str, slot: &str) -> SaveProfileForwardRequest {
    SaveProfileForwardRequest {
        operation_id: id.into(),
        context: context(),
        route: SaveProfileRoute::Select,
        operation: SaveProfileOperation::Select {
            profile_id: SaveProfileId::try_new(slot).expect("slot"),
            baseline: baseline(),
        },
        body: format!("{{\"profile_id\":\"{slot}\"}}").into_bytes(),
    }
}
fn create() -> SaveProfileForwardRequest {
    let user_data = UserDataDescriptor {
        identity: UserDataIdentity::new(1),
        baseline: None,
        provenance: UserDataProvenance {
            owner: "gateway".into(),
            instance_id: "instance".into(),
            operation_id: "create".into(),
            contract: LAUNCH_PROFILE_CONTRACT.into(),
        },
    };
    SaveProfileForwardRequest {
        operation_id: "create".into(),
        context: context(),
        route: SaveProfileRoute::CreateDisposable,
        operation: SaveProfileOperation::CreateDisposable {
            launch_profile: LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, user_data.identity)
                .expect("binding"),
            user_data,
        },
        body: b"{}".to_vec(),
    }
}
fn intent(request: SaveProfileForwardRequest) -> SaveProfileOperationRecord {
    SaveProfileOperationRecord {
        operation_id: request.operation_id.clone(),
        request,
        status: SaveProfileStatus::Accepted,
        result: None,
    }
}

#[derive(Default)]
struct RecordingMod {
    effects: usize,
    lookups: usize,
    receipts: BTreeMap<String, SaveProfileForwardResponse>,
}
#[derive(Clone, Default)]
struct Port(Rc<RefCell<RecordingMod>>);
impl SaveProfileForwardingPort for Port {
    fn forward(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
        let (profile_id, user_data) = match &request.operation {
            SaveProfileOperation::Select { profile_id, .. } => (Some(profile_id.clone()), None),
            SaveProfileOperation::CreateDisposable { user_data, .. } => {
                (None, Some(user_data.clone()))
            }
            _ => return Err(SaveProfileTransportFault::RejectedBeforeWrite),
        };
        let receipt = SaveProfileForwardResponse {
            operation_id: request.operation_id.clone(),
            context: request.context,
            route: request.route,
            status: SaveProfileStatus::Settled,
            body: b"{}".to_vec(),
            profile_id,
            baseline: Some(baseline()),
            user_data,
            reason: None,
        };
        let mut owner = self.0.borrow_mut();
        owner.effects += 1;
        owner.receipts.insert(request.operation_id, receipt);
        Err(SaveProfileTransportFault::TimeoutAfterWrite)
    }
    fn lookup(
        &mut self,
        request: SaveProfileForwardRequest,
    ) -> Result<Option<SaveProfileForwardResponse>, SaveProfileTransportFault> {
        let mut owner = self.0.borrow_mut();
        owner.lookups += 1;
        Ok(owner.receipts.get(&request.operation_id).cloned())
    }
}

#[test]
fn lost_create_and_select_reopen_lookup_original_receipt_without_another_effect() {
    for request in [create(), select("select", "slot")] {
        let journal = Journal::new();
        let port = Port::default();
        {
            let mut ledger = SaveProfileLedger::new(8, authority(), port.clone(), journal.open())
                .expect("ledger");
            assert_eq!(
                ledger
                    .submit(request.clone(), 0, false)
                    .expect("submit")
                    .status,
                SaveProfileStatus::Unknown
            );
        }
        let mut ledger = SaveProfileLedger::new(8, authority(), port.clone(), journal.open())
            .expect("reopened ledger");
        assert_eq!(
            ledger
                .submit(request.clone(), 0, false)
                .expect("duplicate")
                .status,
            SaveProfileStatus::Unknown
        );
        let result = ledger
            .reconcile(context(), &request.operation_id, 0)
            .expect("lookup");
        assert_eq!(result.status, SaveProfileStatus::Settled);
        assert_eq!(result.baseline, Some(baseline()));
        assert_eq!(port.0.borrow().effects, 1);
        assert_eq!(port.0.borrow().lookups, 1);
        assert_eq!(
            ledger.submit(request.clone(), 0, false).expect("replay"),
            result
        );
        let mut stale = request.clone();
        stale.context.lease_epoch += 1;
        assert!(matches!(
            ledger.submit(stale, 0, false),
            Err(SaveProfileLedgerError::Fence(_))
        ));
        let mut changed = request;
        changed.body.push(b' ');
        assert_eq!(
            ledger.submit(changed, 0, false),
            Err(SaveProfileLedgerError::OperationConflict)
        );
        assert_eq!(port.0.borrow().effects, 1);
    }
}

#[test]
fn crash_before_dispatch_preserves_pending_identity_without_blind_retry() {
    let journal = Journal::new();
    let request = select("before", "slot");
    journal
        .open()
        .insert(intent(request.clone()))
        .expect("durable intent");
    let port = Port::default();
    let mut ledger =
        SaveProfileLedger::new(8, authority(), port.clone(), journal.open()).expect("ledger");
    assert_eq!(
        ledger.submit(request, 0, false),
        Err(SaveProfileLedgerError::OperationInProgress)
    );
    assert_eq!(
        ledger
            .reconcile(context(), "before", 0)
            .expect("lookup")
            .status,
        SaveProfileStatus::Accepted
    );
    assert_eq!(port.0.borrow().effects, 0);
}

#[test]
fn latest_selection_is_restored_in_sequence_order_and_foreign_authority_is_rejected() {
    let journal = Journal::new();
    let port = Port::default();
    {
        let mut ledger =
            SaveProfileLedger::new(8, authority(), port.clone(), journal.open()).expect("ledger");
        for (id, slot) in [("z-first", "slot1"), ("a-second", "slot2")] {
            ledger.submit(select(id, slot), 0, false).expect("submit");
            ledger.reconcile(context(), id, 0).expect("settle");
        }
    }
    let ledger =
        SaveProfileLedger::new(8, authority(), port.clone(), journal.open()).expect("reopen");
    assert_eq!(ledger.selected().map(SaveProfileId::as_str), Some("slot2"));
    drop(ledger);
    let mut foreign = authority();
    foreign.instance_id = "foreign".into();
    assert!(SaveProfileLedger::new(8, foreign, port, journal.open()).is_err());
}

#[cfg(unix)]
#[test]
fn replacing_coordinator_lock_fences_prior_connection_after_actual_reopen() {
    let journal = Journal::new();
    let mut first = journal.open();
    first.insert(intent(select("one", "slot"))).expect("intent");
    let mut lock = journal
        .path()
        .canonicalize()
        .expect("canonical")
        .into_os_string();
    lock.push(".save-profile-operations.lock");
    std::fs::remove_file(lock).expect("inject lock replacement");
    let mut second = journal.open();
    assert_eq!(
        first.insert(intent(select("stale", "slot"))),
        Err(SaveProfileLedgerError::OperationConflict)
    );
    assert!(first.list().is_err());
    assert_eq!(second.list().expect("fresh").len(), 1);
}

#[test]
fn journal_rejects_ephemeral_paths_concurrent_owner_and_stale_owner_updates() {
    for path in ["", ":memory:", "file:test?mode=memory"] {
        assert!(SqliteSaveProfileOperationStore::open(path).is_err());
    }
    let journal = Journal::new();
    let mut first = journal.open();
    let record = intent(select("one", "slot"));
    first.insert(record.clone()).expect("insert");
    assert!(SqliteSaveProfileOperationStore::open(journal.path()).is_err());
    let db = rusqlite::Connection::open(journal.path()).expect("fault injector");
    db.execute(
        "UPDATE save_profile_operation_owner SET token=zeroblob(16)",
        [],
    )
    .expect("supersede");
    assert!(first.list().is_err());
    assert!(first.insert(intent(select("two", "slot"))).is_err());
    let mut changed = record;
    changed.result = Some(SaveProfileResult {
        operation_id: "one".into(),
        route: SaveProfileRoute::Select,
        status: SaveProfileStatus::Unknown,
        body: vec![],
        profile_id: None,
        baseline: None,
        user_data: None,
        guidance: RecoveryGuidance::for_status(SaveProfileStatus::Unknown),
    });
    changed.status = SaveProfileStatus::Unknown;
    assert!(first.update(changed).is_err());
}

#[test]
fn corrupt_keys_unknown_fields_duplicates_oversize_and_invalid_results_fail_closed() {
    for corruption in 0..6 {
        let journal = Journal::new();
        journal
            .open()
            .insert(intent(select("one", "slot")))
            .expect("insert");
        let db = rusqlite::Connection::open(journal.path()).expect("injector");
        let mut body: Vec<u8> = db
            .query_row(
                "SELECT body FROM save_profile_operation_records",
                [],
                |row| row.get(0),
            )
            .expect("body");
        match corruption {
            0 => {
                db.execute(
                    "UPDATE save_profile_operation_records SET instance_id='foreign'",
                    [],
                )
                .expect("key");
            }
            1 => {
                body.splice(1..1, b"\"extra\":true,".iter().copied());
            }
            2 => {
                body.splice(1..1, b"\"version\":1,".iter().copied());
            }
            3 => {
                body = vec![b' '; 160 * 1024 + 1];
            }
            4 => {
                body = String::from_utf8(body)
                    .expect("JSON")
                    .replace("\"Accepted\"", "\"Settled\"")
                    .into_bytes();
            }
            5 => {
                body = String::from_utf8(body)
                    .expect("JSON")
                    .replace("\"lease_epoch\":1", "\"lease_epoch\":1,\"surprise\":true")
                    .into_bytes();
            }
            _ => unreachable!(),
        }
        if corruption != 0 {
            db.execute("UPDATE save_profile_operation_records SET body=?1", [body])
                .expect("corrupt");
        }
        assert!(
            SqliteSaveProfileOperationStore::open(journal.path()).is_err(),
            "case{corruption}"
        );
    }
}

#[test]
fn record_capacity_and_conflicting_updates_fail_before_forwarding() {
    let journal = Journal::new();
    let mut store = journal.open();
    for index in 0..SAVE_PROFILE_OPERATION_CAPACITY {
        store
            .insert(intent(select(&format!("op-{index}"), "slot")))
            .expect("bounded insert");
    }
    assert_eq!(
        store.insert(intent(select("overflow", "slot"))),
        Err(SaveProfileLedgerError::CapacityExceeded)
    );
    assert_eq!(
        store.insert(intent(select("op-0", "slot"))),
        Err(SaveProfileLedgerError::OperationConflict)
    );
    let mut changed = intent(select("op-0", "other"));
    changed.status = SaveProfileStatus::Unknown;
    changed.result = Some(SaveProfileResult {
        operation_id: "op-0".into(),
        route: SaveProfileRoute::Select,
        status: SaveProfileStatus::Unknown,
        body: vec![],
        profile_id: None,
        baseline: None,
        user_data: None,
        guidance: RecoveryGuidance::for_status(SaveProfileStatus::Unknown),
    });
    assert_eq!(
        store.update(changed),
        Err(SaveProfileLedgerError::OperationConflict)
    );
}
