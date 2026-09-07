// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_gateway::MAX_HOST_LEASE_FRAME_BYTES;

use super::super::host_lease_control_crypto as crypto;
use super::{HostLeaseFrameError, HostLeaseKind, parse_request};

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
