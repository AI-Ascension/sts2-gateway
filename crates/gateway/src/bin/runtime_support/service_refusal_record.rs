// SPDX-License-Identifier: MIT

//! The server-side record of a refused header.
//!
//! `sts2-gateway#113` made the `unsupported_header` body name the header it
//! refused, but the serving process then discards that body once it is written
//! to the socket. Everything the process retained was
//! `metrics.malformed_rejected()`, a bare counter that is identical for a
//! request this gateway could not parse and for one whose headers were simply
//! not on the allow-list. So the one fact that identifies a recurrence was
//! thrown away, and an intermittent `unsupported_header` could only be
//! adjudicated by re-running the episode (AI-Ascension/sts2-harness#541).
//!
//! This module is the record. It is deliberately not a counter and not a
//! buffer: it writes one line to the process's standard error and holds
//! nothing afterwards.

/// Records that `name` was the header the gateway refused.
///
/// The only argument is the header *name*. A header value may be a bearer
/// credential, so no value is read, copied, or formatted here — this function
/// is never given one, and it takes `&str` rather than the request so that a
/// caller cannot pass the whole header set by mistake. `first_rejected_header`
/// constrains the name to the RFC 7230 token charset at parse time, so the name
/// cannot carry a delimiter and cannot forge a second log line.
///
/// Standard error is the record surface because it is the one stream this
/// process already uses for out-of-band diagnostics and the one the harness
/// already inherits when it starts the gateway
/// (`runtime_v3_game_information_entry_live_peers.rs` spawns the pinned gateway
/// with `Stdio::inherit`), so the line lands in the run artifact. No new
/// dependency, no ring buffer, no counter, and no state: the cost is one
/// write on a path that is already refusing the request, and nothing is
/// retained that a later request could read.
pub(super) fn record_rejected_header(name: &str) {
    write_record(&rendered_line(name));
}

/// Where the record goes, selected at compile time.
///
/// The served binary writes the process's standard error. A `cfg(test)` build
/// appends to a per-thread sink instead, because the in-process refusal path is
/// exercised inside the test process, where the real stream cannot be read back.
/// The two arms are mutually exclusive, so the served binary has no test
/// behaviour and the tests do not write to the real stream.
#[cfg(not(test))]
fn write_record(line: &str) {
    eprintln!("{line}");
}

#[cfg(test)]
fn write_record(line: &str) {
    RECORDED.with(|recorded| recorded.borrow_mut().push(line.to_owned()));
}

#[cfg(test)]
thread_local! {
    /// The lines this thread recorded, in order.
    static RECORDED: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The lines this thread has recorded, and clears them.
///
/// Clearing on read keeps two assertions in one test independent, so a test
/// that checks "one refusal, one record" is not satisfied by a record an
/// earlier assertion in the same thread left behind.
#[cfg(test)]
pub(super) fn take_recorded() -> Vec<String> {
    RECORDED.with(|recorded| std::mem::take(&mut *recorded.borrow_mut()))
}

/// The exact line the record is written as.
///
/// Split out from the write so the line's shape can be asserted directly. A
/// name that somehow arrived unvalidated would have its line breaks stripped
/// rather than emitted, so even an invalid input cannot make the record span
/// two lines or forge a second entry in the stream.
pub(super) fn rendered_line(name: &str) -> String {
    let single_line = name
        .chars()
        .filter(|character| !matches!(character, '\n' | '\r'))
        .collect::<String>();
    format!("gateway refused unsupported header: {single_line}")
}
