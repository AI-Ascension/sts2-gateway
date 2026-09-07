// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::fs;
use std::path::Path;

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_gateway::MAX_HOST_LEASE_FRAME_BYTES;

use super::super::host_lease_control_crypto as crypto;
use super::super::strict_json;
use super::{HostLeaseFrameError, HostLeaseKind, ack_context, parse_request, parse_response};

const ARTIFACT_ROOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contract-artifact/host-lease-control-v1"
);
const SCHEMA_DIGEST: &str = "e22faf0f7d3cd313a007b65e52058b3c255153d5778dd8124055c283adf977f9";
const MANIFEST_SHA256: &str = "17552a4cd001ce9e622535fafdedfc8d5d7f9b93ffdfa5696ae7689d3aaf84b1";
const PROFILE_SHA256: &str = "1dd25a5520c655fb8107475c1a510830bcce4a157f726e9f7b15122070ffddef";
const VECTORS_SHA256: &str = "942c5c8ad07705ae43ff5fc44309b1aaa474bd7cb48152f2384117922a4abb30";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|digit| char::from(b"0123456789abcdef"[digit as usize]))
        .collect()
}

fn artifact(path: &str) -> Vec<u8> {
    fs::read(Path::new(ARTIFACT_ROOT).join(path)).expect("committed host lease artifact")
}

fn vectors() -> Value {
    serde_json::from_slice(&artifact("proof-vectors.json")).expect("valid proof vectors")
}

fn test_key() -> Vec<u8> {
    let vector_data = vectors();
    let hex = vector_data["test_key_hex"].as_str().expect("test key hex");
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("test key byte"))
        .collect()
}

#[test]
fn raw_decoder_rejects_duplicate_members_trailing_bytes_and_invalid_utf8() {
    for bytes in [
        br#"{"contract":"a","contract":"b"}"#.as_slice(),
        br#"{} trailing"#.as_slice(),
        &[b'{', 0xff, b'}'],
    ] {
        assert_eq!(
            parse_request(bytes, HostLeaseKind::Install),
            Err(HostLeaseFrameError::Invalid)
        );
    }
}

#[test]
fn raw_decoder_rejects_oversized_frames_before_json_parsing() {
    let bytes = vec![b' '; MAX_HOST_LEASE_FRAME_BYTES + 1];
    assert_eq!(
        parse_request(&bytes, HostLeaseKind::Install),
        Err(HostLeaseFrameError::Oversized)
    );
}

#[test]
fn proof_is_bound_to_operation_domain_and_secret() {
    let frame = json!({
        "auth": { "proof": null },
        "kind": "lease_install_request"
    });
    let secret = [0x11_u8; 32];
    let install = crypto::proof_for_frame(&frame, HostLeaseKind::Install.request_domain(), &secret);
    let renew = crypto::proof_for_frame(&frame, HostLeaseKind::Renew.request_domain(), &secret);
    let mut alternate_secret = secret;
    alternate_secret[0] ^= 1;
    let alternate = crypto::proof_for_frame(
        &frame,
        HostLeaseKind::Install.request_domain(),
        &alternate_secret,
    );
    assert_eq!(install.len(), 64);
    assert_ne!(install, renew);
    assert_ne!(install, alternate);
}

#[test]
fn published_host_lease_artifact_digests_are_pinned() {
    assert_eq!(sha256_hex(&artifact("frame.schema.json")), SCHEMA_DIGEST);
    assert_eq!(sha256_hex(&artifact("manifest.json")), MANIFEST_SHA256);
    assert_eq!(sha256_hex(&artifact("PROOF_PROFILE.md")), PROFILE_SHA256);
    assert_eq!(sha256_hex(&artifact("proof-vectors.json")), VECTORS_SHA256);
    assert_eq!(
        serde_json::from_slice::<Value>(&artifact("manifest.json")).expect("manifest")["schema_digest"],
        SCHEMA_DIGEST
    );
}

#[test]
fn published_hcj1_canonical_cases_match_exact_bytes() {
    for case in vectors()["canonical_cases"]
        .as_array()
        .expect("canonical cases")
    {
        let canonical = crypto::canonical_hcj1(&case["input"]).expect("canonical case");
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(&canonical),
            case["canonical_utf8_base64"]
        );
        assert_eq!(sha256_hex(&canonical), case["canonical_sha256"]);
    }
}

#[test]
fn published_host_lease_proofs_match_all_nine_vectors() {
    let key = test_key();
    for case in vectors()["frame_proof_cases"]
        .as_array()
        .expect("frame proof cases")
    {
        let fixture = case["fixture"].as_str().expect("fixture path");
        let bytes = artifact(fixture);
        assert_eq!(sha256_hex(&bytes), case["fixture_sha256"]);
        let frame: Value = serde_json::from_slice(&bytes).expect("valid fixture");
        let mut unsigned = frame.clone();
        unsigned["auth"]
            .as_object_mut()
            .expect("auth object")
            .remove("proof");
        let canonical = crypto::canonical_hcj1(&unsigned).expect("canonical frame");
        let domain = case["domain"].as_str().expect("proof domain");
        let mut message = domain.as_bytes().to_vec();
        message.push(0);
        message.extend_from_slice(&canonical);
        assert_eq!(sha256_hex(&canonical), case["canonical_sha256"]);
        assert_eq!(sha256_hex(&message), case["signed_message_sha256"]);
        assert_eq!(crypto::proof_for_frame(&frame, domain, &key), case["proof"]);
    }
}

#[test]
fn response_requires_the_pinned_host_principal_and_dedicated_key() {
    let key = test_key();
    let mut frame: Value =
        serde_json::from_slice(&artifact("fixtures/valid/lease-install-response.json"))
            .expect("valid response fixture");
    let proof =
        crypto::proof_for_frame(&frame, HostLeaseKind::Install.acknowledgment_domain(), &key);
    frame["auth"]["proof"] = proof.into();
    let bytes = serde_json::to_vec(&frame).expect("response bytes");
    let principal = "00000000-0000-4000-8000-00000000000a";
    assert!(parse_response(&bytes, HostLeaseKind::Install, &key, principal).is_ok());
    assert!(parse_response(&bytes, HostLeaseKind::Install, &[0x01; 32], principal).is_err());

    frame["actor"]["principal_id"] = "00000000-0000-4000-8000-00000000000b".into();
    frame["auth"]["principal_id"] = "00000000-0000-4000-8000-00000000000b".into();
    let wrong_principal_proof =
        crypto::proof_for_frame(&frame, HostLeaseKind::Install.acknowledgment_domain(), &key);
    frame["auth"]["proof"] = wrong_principal_proof.into();
    let wrong_principal = serde_json::to_vec(&frame).expect("wrong principal response bytes");
    assert!(parse_response(&wrong_principal, HostLeaseKind::Install, &key, principal).is_err());
}

#[test]
fn published_ack_fixtures_keep_operation_specific_expiry_shape() {
    let key = test_key();
    let principal = "00000000-0000-4000-8000-00000000000a";
    for (fixture, kind) in [
        (
            "fixtures/valid/lease-install-response.json",
            HostLeaseKind::Install,
        ),
        (
            "fixtures/valid/lease-install-duplicate-response.json",
            HostLeaseKind::Install,
        ),
        (
            "fixtures/valid/lease-renew-response.json",
            HostLeaseKind::Renew,
        ),
        (
            "fixtures/valid/lease-renew-duplicate-response.json",
            HostLeaseKind::Renew,
        ),
        (
            "fixtures/valid/lease-revoke-response.json",
            HostLeaseKind::Revoke,
        ),
        (
            "fixtures/valid/lease-revoke-duplicate-response.json",
            HostLeaseKind::Revoke,
        ),
    ] {
        let mut frame: Value =
            serde_json::from_slice(&artifact(fixture)).expect("response fixture");
        frame["auth"]["proof"] =
            crypto::proof_for_frame(&frame, kind.acknowledgment_domain(), &key).into();
        let bytes = serde_json::to_vec(&frame).expect("response bytes");
        let parsed = parse_response(&bytes, kind, &key, principal).expect("signed response");
        let ack = ack_context(&parsed).expect("ack context");
        match kind {
            HostLeaseKind::Install => {
                assert_eq!(ack.renew_sequence, None, "{fixture}");
                assert!(ack.expires_at.is_some(), "{fixture}");
            }
            HostLeaseKind::Renew => {
                assert!(ack.renew_sequence.is_some(), "{fixture}");
                assert!(ack.expires_at.is_some(), "{fixture}");
            }
            HostLeaseKind::Revoke => {
                assert_eq!(ack.renew_sequence, None, "{fixture}");
                assert_eq!(ack.expires_at, None, "{fixture}");
            }
        }
    }
}

#[test]
fn delayed_signed_ack_remains_authenticated_but_is_not_fresh() {
    let key = test_key();
    let principal = "00000000-0000-4000-8000-00000000000a";
    let mut frame: Value =
        serde_json::from_slice(&artifact("fixtures/valid/lease-install-response.json"))
            .expect("valid response fixture");
    frame["payload"]["ack"]["recorded_at"] = "2026-09-07T00:00:30Z".into();
    frame["auth"]["proof"] =
        crypto::proof_for_frame(&frame, HostLeaseKind::Install.acknowledgment_domain(), &key)
            .into();
    let bytes = serde_json::to_vec(&frame).expect("response bytes");
    let parsed = parse_response(&bytes, HostLeaseKind::Install, &key, principal)
        .expect("delayed response is still cryptographically valid");
    let ack = ack_context(&parsed).expect("ack context");
    assert_eq!(
        ack.recorded_at,
        crypto::parse_timestamp_millis("2026-09-07T00:00:30Z").expect("timestamp")
    );
    assert!(parse_response(&bytes, HostLeaseKind::Install, &[0x01; 32], principal).is_err());
    let wrong_principal = "00000000-0000-4000-8000-00000000000b";
    assert!(parse_response(&bytes, HostLeaseKind::Install, &key, wrong_principal).is_err());
}

#[test]
fn published_raw_hcj1_rejection_cases_fail_before_normalization() {
    for raw in vectors()["reject_raw_json"]
        .as_array()
        .expect("raw rejection cases")
    {
        let raw = raw.as_str().expect("raw JSON string");
        assert!(
            strict_json::parse_hcj1(raw.as_bytes()).is_err(),
            "accepted noncanonical raw input: {raw}"
        );
    }
}
