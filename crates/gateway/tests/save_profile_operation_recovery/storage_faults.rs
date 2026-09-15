// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn terminal_receipt_cannot_be_overwritten_or_reported_as_changed() {
    let journal = Journal::new();
    let mut ledger =
        SaveProfileLedger::new(8, authority(), Port::default(), journal.open()).expect("ledger");
    ledger
        .submit(select("one", "slot"), 0, false)
        .expect("submit");
    ledger.reconcile(context(), "one", 0).expect("settle");
    let original = ledger.operation("instance", "one").expect("record").clone();
    let mut store = ledger.into_store();
    let mut changed = original.clone();
    changed.result.as_mut().expect("result").baseline = Some(ProfileBaseline {
        identity: "different".into(),
        digest: "b".repeat(64),
    });
    assert_eq!(
        store.update(changed),
        Err(SaveProfileLedgerError::OperationConflict)
    );
    assert_eq!(store.list().expect("retained"), vec![original]);
}

#[test]
fn suppression_and_alteration_triggers_block_admission_before_forwarding() {
    for action in [
        "CREATE TRIGGER suppress BEFORE INSERT ON save_profile_operation_records BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER alter_row AFTER INSERT ON save_profile_operation_records BEGIN UPDATE save_profile_operation_records SET body=x'00'; END",
        "CREATE TRIGGER erase_row AFTER INSERT ON save_profile_operation_records BEGIN DELETE FROM save_profile_operation_records; END",
        "CREATE VIEW unexpected AS SELECT * FROM save_profile_operation_records",
    ] {
        let journal = Journal::new();
        let port = Port::default();
        let mut ledger =
            SaveProfileLedger::new(8, authority(), port.clone(), journal.open()).expect("ledger");
        let db = rusqlite::Connection::open(journal.path()).expect("injector");
        db.execute_batch(action).expect("unexpected schema");
        assert!(ledger.submit(select("one", "slot"), 0, false).is_err());
        assert_eq!(port.0.borrow().effects, 0);
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM save_profile_operation_records",
                [],
                |row| row.get(0),
            )
            .expect("rows");
        assert_eq!(count, 0);
        drop(ledger);
        assert!(SqliteSaveProfileOperationStore::open(journal.path()).is_err());
    }
}

#[test]
fn suppressed_and_altered_result_updates_never_acknowledge_a_receipt() {
    for action in [
        "CREATE TRIGGER suppress BEFORE UPDATE ON save_profile_operation_records BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER alter_row AFTER UPDATE ON save_profile_operation_records BEGIN UPDATE save_profile_operation_records SET body=x'00'; END",
    ] {
        let journal = Journal::new();
        let port = Port::default();
        let mut ledger =
            SaveProfileLedger::new(8, authority(), port.clone(), journal.open()).expect("ledger");
        ledger
            .submit(select("one", "slot"), 0, false)
            .expect("unknown");
        let db = rusqlite::Connection::open(journal.path()).expect("injector");
        let before: Vec<u8> = db
            .query_row(
                "SELECT body FROM save_profile_operation_records",
                [],
                |row| row.get(0),
            )
            .expect("before");
        db.execute_batch(action).expect("trigger");
        assert!(ledger.reconcile(context(), "one", 0).is_err());
        let after: Vec<u8> = db
            .query_row(
                "SELECT body FROM save_profile_operation_records",
                [],
                |row| row.get(0),
            )
            .expect("after");
        assert_eq!(before, after);
        assert_eq!(port.0.borrow().effects, 1);
    }
}

#[test]
fn owner_upsert_trigger_is_rejected_without_claiming_journal_ownership() {
    let journal = Journal::new();
    drop(journal.open());
    let db = rusqlite::Connection::open(journal.path()).expect("injector");
    let before: Vec<u8> = db
        .query_row(
            "SELECT token FROM save_profile_operation_owner",
            [],
            |row| row.get(0),
        )
        .expect("token");
    db.execute_batch(
        "CREATE TRIGGER suppress BEFORE UPDATE ON save_profile_operation_owner BEGIN SELECT RAISE(IGNORE); END"
    ).expect("trigger");
    assert!(SqliteSaveProfileOperationStore::open(journal.path()).is_err());
    let after: Vec<u8> = db
        .query_row(
            "SELECT token FROM save_profile_operation_owner",
            [],
            |row| row.get(0),
        )
        .expect("retained token");
    assert_eq!(before, after);
}
