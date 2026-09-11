// SPDX-License-Identifier: MIT

use super::{optional_value, safe_identity};

pub(super) fn peer_binding_from_environment() -> Result<(Option<String>, Option<String>), String> {
    let token = optional_value("STS2_COOP_NATIVE_PEER_TOKEN")?;
    let peer = optional_value("STS2_COOP_NATIVE_PEER_ID")?;
    if token.is_some() != peer.is_some() {
        return Err(String::from(
            "STS2_COOP_NATIVE_PEER_TOKEN and STS2_COOP_NATIVE_PEER_ID must be configured together",
        ));
    }
    if token
        .as_deref()
        .is_some_and(|value| !valid_private_route_credential(value))
    {
        return Err(String::from(
            "STS2_COOP_NATIVE_PEER_TOKEN is empty, unsafe, or oversized",
        ));
    }
    if peer
        .as_deref()
        .is_some_and(|value| !valid_canonical_peer_identity(value))
    {
        return Err(String::from(
            "STS2_COOP_NATIVE_PEER_ID must be a canonical coop-native-v1 peer identity",
        ));
    }
    Ok((token, peer))
}

// This credential never enters the v1 schema. Keep its private configuration policy separate
// from the wider, artifact-defined peer-identity grammar below.
fn valid_private_route_credential(value: &str) -> bool {
    safe_identity(value)
}

// Mirrors `coop-native-v1` `$defs.peer_identity`: `peer:` plus 5..=507 ASCII identifier bytes.
// In particular, `..` is valid here because this value is an opaque protocol identity, never a
// filesystem path or route fragment.
fn valid_canonical_peer_identity(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("peer:") else {
        return false;
    };
    (5..=507).contains(&suffix.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'/' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::{valid_canonical_peer_identity, valid_private_route_credential};

    #[test]
    fn route_credential_stays_private_and_bounded() {
        assert!(valid_private_route_credential("credential-01"));
        assert!(!valid_private_route_credential("credential..01"));
        assert!(!valid_private_route_credential("credential with space"));
        assert!(!valid_private_route_credential(&"x".repeat(129)));
    }

    #[test]
    fn canonical_peer_identity_matches_the_frozen_v1_grammar() {
        assert!(valid_canonical_peer_identity("peer:abcde"));
        assert!(valid_canonical_peer_identity("peer:alpha..beta"));
        assert!(valid_canonical_peer_identity(&format!(
            "peer:{}",
            "a".repeat(507)
        )));
        assert!(!valid_canonical_peer_identity("peer:abcd"));
        assert!(!valid_canonical_peer_identity("host:abcde"));
        assert!(!valid_canonical_peer_identity(&format!(
            "peer:{}",
            "a".repeat(508)
        )));
        assert!(!valid_canonical_peer_identity("peer:alpha beta"));
    }
}
