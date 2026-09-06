// SPDX-License-Identifier: MIT

use super::*;

const ROSTER: &str = r#"[{"peer_id":"local-1","role":"local"},{"peer_id":"ally-1","role":"ally"}]"#;

fn report(peer: &str, generation: u64, connected: bool) -> PeerReport {
    PeerReport {
        peer_id: peer.to_owned(),
        generation,
        connected,
    }
}

#[test]
fn missing_converged_disagreed_disconnected_and_recovered_reports() -> Result<(), String> {
    let mut reports = CoopReports::from_roster(ROSTER)?;
    let now = Instant::now();
    assert_eq!(
        reports.snapshot(now).2["missing_peers"],
        json!(["local-1", "ally-1"])
    );
    reports.report(report("local-1", 4, true), now)?;
    assert_eq!(reports.snapshot(now).2["status"], "disconnected");
    reports.report(report("ally-1", 4, true), now)?;
    assert_eq!(reports.snapshot(now).0, 4);
    assert_eq!(reports.snapshot(now).2["status"], "synchronized");
    reports.report(report("local-1", 5, true), now)?;
    assert_eq!(reports.snapshot(now).0, 4);
    assert_eq!(reports.snapshot(now).2["status"], "disagreement");
    reports.report(report("ally-1", 4, false), now)?;
    assert_eq!(reports.snapshot(now).2["status"], "disconnected");
    reports.report(report("ally-1", 5, true), now)?;
    assert_eq!(reports.snapshot(now).0, 5);
    assert_eq!(reports.snapshot(now).2["status"], "synchronized");
    Ok(())
}

#[test]
fn expiry_requires_fresh_reports_from_every_peer() -> Result<(), String> {
    let mut reports = CoopReports::from_roster(ROSTER)?;
    let now = Instant::now();
    reports.report(report("local-1", 4, true), now)?;
    reports.report(report("ally-1", 4, true), now)?;
    assert_eq!(
        reports.snapshot(now + REPORT_LIFETIME).2["status"],
        "disconnected"
    );
    let later = now + REPORT_LIFETIME;
    reports.report(report("local-1", 5, true), later)?;
    assert_eq!(
        reports.snapshot(later).2["missing_peers"],
        json!(["ally-1"])
    );
    assert_eq!(reports.snapshot(later).0, 4);
    reports.report(report("ally-1", 5, true), later)?;
    assert_eq!(reports.snapshot(later).2["status"], "synchronized");
    assert_eq!(reports.snapshot(later).0, 5);
    assert_eq!(reports.snapshot(now).2["status"], "disconnected");
    Ok(())
}

#[test]
fn invalid_reports_never_change_the_ledger() -> Result<(), String> {
    let mut reports = CoopReports::from_roster(ROSTER)?;
    let now = Instant::now();
    reports.report(report("local-1", 4, true), now)?;
    reports.report(report("ally-1", 4, true), now)?;
    let before = reports.snapshot(now);
    for input in [
        report("foreign", 5, true),
        report("local-1", 3, true),
        report("ally-1", MAX_GENERATION + 1, true),
    ] {
        assert!(reports.report(input, now).is_err());
        assert_eq!(reports.snapshot(now), before);
    }
    reports.report(report("local-1", 5, true), now)?;
    assert!(reports.report(report("local-1", 4, true), now).is_err());
    Ok(())
}

#[test]
fn roster_and_report_shapes_are_closed_and_bounded() {
    for text in [
        "[]",
        "null",
        r#"[{"peer_id":"one","role":"local"}]"#,
        r#"[{"peer_id":"one","role":"local"},{"peer_id":"one","role":"ally"}]"#,
        r#"[{"peer_id":"one","role":"local"},{"peer_id":"two","role":"local"}]"#,
        r#"[{"peer_id":"one","role":"ally"},{"peer_id":"two","role":"ally"}]"#,
        r#"[{"peer_id":"one","role":"local","extra":0},{"peer_id":"two","role":"ally"}]"#,
    ] {
        assert!(CoopReports::from_roster(text).is_err(), "{text}");
    }
    for text in [
        r#"{"peer_id":"local-1","generation":4,"connected":true,"extra":0}"#,
        r#"{"peer_id":"local-1","generation":4,"generation":4,"connected":true}"#,
        r#"{"peer_id":"local-1","generation":4.0,"connected":true}"#,
        r#"{"peer_id":"local-1","generation":4}"#,
    ] {
        assert!(serde_json::from_str::<PeerReport>(text).is_err(), "{text}");
    }
}
