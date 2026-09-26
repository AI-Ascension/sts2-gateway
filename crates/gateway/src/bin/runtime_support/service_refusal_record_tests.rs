// SPDX-License-Identifier: MIT

//! Tests for the server-side record of a refused header.
//!
//! These pin the record itself rather than the response body. A body assertion
//! cannot tell this change from `sts2-gateway#113`, because the body already
//! named the header and the process then discarded it
//! (AI-Ascension/sts2-harness#541).

use super::test_support::{authenticated_request, test_service};

/// The in-process refusal path is `handle_request` →
/// `handle_request_with_cancellation` → `request_rejection`, the same function
/// the socket path calls. Recording inside `request_rejection` is what makes
/// both paths record, and this pins the in-process half of that claim against
/// the record itself rather than against the response body.
#[test]
fn the_in_process_refusal_path_records_the_name_it_returns() -> Result<(), String> {
    let recorded = super::refusal_record::take_recorded();
    assert!(
        recorded.is_empty(),
        "a thread that has refused nothing must have recorded nothing: {recorded:?}"
    );
    let mut service = test_service()?;
    let mut request = authenticated_request("/health/ready");
    request
        .headers
        .insert("x-sts2-in-process".to_owned(), String::from("secret-value"));
    let (status, bytes) = service.handle_request(&request);
    assert_eq!(status, 400);
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    assert_eq!(value["error_code"], "unsupported_header");
    // One refusal, one record, and the record names the same header the client
    // was just told about.
    assert_eq!(
        super::refusal_record::take_recorded(),
        vec![String::from(
            "gateway refused unsupported header: x-sts2-in-process"
        )]
    );
    assert_eq!(value["rejected_header"], "x-sts2-in-process");
    // The value stayed in the request; it is in neither the body nor the record.
    assert!(
        !String::from_utf8_lossy(&bytes).contains("secret-value"),
        "the in-process refusal must not echo the value: {}",
        String::from_utf8_lossy(&bytes),
    );
    Ok(())
}

/// An admitted request must leave no refused-header record at all, so the
/// record cannot be read as "this process once refused something".
#[test]
fn an_admitted_request_records_no_refused_header_name() -> Result<(), String> {
    let _ = super::refusal_record::take_recorded();
    let mut service = test_service()?;
    // `/health/ready` with only allow-listed headers is not refused: it is
    // forwarded downstream, which this fixture cannot reach, but it does not
    // take the `unsupported_header` path.
    let request = authenticated_request("/health/live");
    let (status, _) = service.handle_request(&request);
    assert_eq!(status, 200, "an allow-listed request must be served");
    assert!(
        super::refusal_record::take_recorded().is_empty(),
        "an admitted request must not record a refused header name"
    );
    Ok(())
}

/// A recorded name cannot forge a second log line.
///
/// A name is token-charset by `valid_header`, so it cannot contain CR, LF, or
/// a colon. This asserts the rendering invariant that makes that safe, and
/// checks it holds even for an unvalidated string.
#[test]
fn a_recorded_name_cannot_forge_a_second_log_line() -> Result<(), String> {
    for name in ["accept", "x-sts2-a.b_c~d", "a1"] {
        let line = super::refusal_record::rendered_line(name);
        assert_eq!(line, format!("gateway refused unsupported header: {name}"));
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "a record must be exactly one line: {line:?}"
        );
        assert_eq!(line.matches(':').count(), 1, "one separator only: {line:?}");
    }
    // Total over arbitrary input: an unvalidated string still cannot inject a
    // newline into the record stream.
    let hostile = super::refusal_record::rendered_line("accept\r\ngateway admitted everything");
    assert!(!hostile.contains('\n'), "{hostile:?}");
    assert!(!hostile.contains('\r'), "{hostile:?}");
    Ok(())
}

/// Each refusal records its own name, so no record is attributable to a
/// different request, and a repeated identical request records identically.
#[test]
fn each_refusal_records_only_its_own_name() -> Result<(), String> {
    let _ = super::refusal_record::take_recorded();
    let mut service = test_service()?;
    for name in ["x-sts2-first", "x-sts2-second", "x-sts2-first"] {
        let mut request = authenticated_request("/health/ready");
        request
            .headers
            .insert(name.to_owned(), String::from("value"));
        let (status, bytes) = service.handle_request(&request);
        assert_eq!(status, 400);
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        assert_eq!(value["rejected_header"], name);
    }
    assert_eq!(
        super::refusal_record::take_recorded(),
        vec![
            String::from("gateway refused unsupported header: x-sts2-first"),
            String::from("gateway refused unsupported header: x-sts2-second"),
            String::from("gateway refused unsupported header: x-sts2-first"),
        ],
        "each refusal records its own name, in order, and repeats identically"
    );
    Ok(())
}
