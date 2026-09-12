// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::{Value, json};
use sts2_gateway::{
    ReferenceError, validate_exact_checkpoint_reference, verify_exact_checkpoint_reference_artifact,
};

fn reference() -> Value {
    json!({
        "schema": "ascension.exact_checkpoint_reference.v1",
        "reference_version": "exact-checkpoint-reference-v1",
        "handle": format!("ckpt-h1:{}", "a".repeat(64)),
        "occurrence": "run:alpha:1",
        "boundary_kind": "decision",
        "boundary_phase": "COMBAT",
        "assurance": "restore_verified",
        "restore_verified": true,
    })
}

#[test]
fn copied_artifact_verifies_before_any_route_accepts_a_reference() {
    verify_exact_checkpoint_reference_artifact().expect("copied artifact is consistent");
}

#[test]
fn valid_references_pass_the_boundary() {
    assert!(validate_exact_checkpoint_reference(&reference()).is_ok());
    let mut observation_only = reference();
    observation_only["assurance"] = json!("public_observation_only");
    observation_only["restore_verified"] = json!(false);
    assert!(validate_exact_checkpoint_reference(&observation_only).is_ok());
}

#[test]
fn privileged_members_and_unknown_versions_are_refused() {
    let mut privileged = reference();
    privileged["exact_state_digest"] = json!(format!("asc-state:v1:sha256:{}", "a".repeat(64)));
    assert_eq!(
        validate_exact_checkpoint_reference(&privileged).expect_err("digest member refused"),
        ReferenceError::UnexpectedMember
    );

    let mut future = reference();
    future["reference_version"] = json!("exact-checkpoint-reference-v2");
    assert_eq!(
        validate_exact_checkpoint_reference(&future).expect_err("future version refused"),
        ReferenceError::UnsupportedVersion
    );

    let mut missing = reference();
    missing.as_object_mut().expect("object").remove("handle");
    assert_eq!(
        validate_exact_checkpoint_reference(&missing).expect_err("missing member refused"),
        ReferenceError::MissingMember
    );
}

#[test]
fn malformed_handles_occurrences_boundaries_and_assurance_are_refused() {
    for handle in [
        "ckpt-h1:short".to_owned(),
        format!("ckpt-h1:{}", "A".repeat(64)),
        format!("ckpt:{}", "a".repeat(64)),
    ] {
        let mut value = reference();
        value["handle"] = json!(handle);
        assert_eq!(
            validate_exact_checkpoint_reference(&value).expect_err("handle refused"),
            ReferenceError::InvalidHandle
        );
    }

    let mut bad_occurrence = reference();
    bad_occurrence["occurrence"] = json!("run/alpha");
    assert_eq!(
        validate_exact_checkpoint_reference(&bad_occurrence).expect_err("occurrence refused"),
        ReferenceError::InvalidOccurrence
    );

    let mut bad_boundary = reference();
    bad_boundary["boundary_phase"] = json!("");
    assert_eq!(
        validate_exact_checkpoint_reference(&bad_boundary).expect_err("boundary refused"),
        ReferenceError::InvalidBoundary
    );

    let mut bad_assurance = reference();
    bad_assurance["assurance"] = json!("definitely_restored");
    assert_eq!(
        validate_exact_checkpoint_reference(&bad_assurance).expect_err("assurance refused"),
        ReferenceError::InvalidAssurance
    );

    let mut not_an_object = json!(["reference"]);
    assert_eq!(
        validate_exact_checkpoint_reference(&not_an_object).expect_err("array refused"),
        ReferenceError::NotAnObject
    );
    not_an_object = json!(null);
    assert_eq!(
        validate_exact_checkpoint_reference(&not_an_object).expect_err("null refused"),
        ReferenceError::NotAnObject
    );
}

#[test]
fn assurance_and_restore_flag_must_agree() {
    let mut overclaim = reference();
    overclaim["assurance"] = json!("capture_only");
    assert_eq!(
        validate_exact_checkpoint_reference(&overclaim).expect_err("overclaim refused"),
        ReferenceError::InvalidRestoreFlag
    );

    let mut underclaim = reference();
    underclaim["restore_verified"] = json!(false);
    assert_eq!(
        validate_exact_checkpoint_reference(&underclaim).expect_err("underclaim refused"),
        ReferenceError::InvalidRestoreFlag
    );

    let mut non_boolean = reference();
    non_boolean["restore_verified"] = json!("true");
    assert_eq!(
        validate_exact_checkpoint_reference(&non_boolean).expect_err("non-boolean refused"),
        ReferenceError::InvalidRestoreFlag
    );
}
