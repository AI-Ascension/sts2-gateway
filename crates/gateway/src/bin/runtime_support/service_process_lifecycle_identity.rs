// SPDX-License-Identifier: MIT

//! Deterministic mapping from the attached runtime's string identities to lifecycle numbers.
//!
//! The attached runtime authenticates requests with string instance/caller/session/lease
//! identities, while the lifecycle coordinator's identity space is numeric. The two must be
//! bridged without letting a caller choose either side.
//!
//! The bridge is a domain-separated SHA-256 truncation: each namespace hashes
//! `<domain> + "\0" + <value>` and keeps the leading 64 bits. This is deterministic across
//! restarts and processes, which is required for durable records to keep matching their lease,
//! and it is not caller-selectable, because the input is gateway configuration that the request
//! has already been fenced against.
//!
//! Collision resistance here is a convenience, not the security boundary. A fence decision
//! compares the complete five-field proof, and every request must additionally pass the
//! configured string fence, so a forged match would have to collide in all five namespaces at
//! once while also presenting the correct configured strings.

use sts2_gateway::{CallerId, InstanceId, LeaseEpoch, LeaseId, LeaseProof, SessionId, sha256_hex};

/// Derives the numeric lifecycle instance identity for a configured instance id.
pub(super) fn instance_id(configured: &str) -> InstanceId {
    InstanceId::new(digest("instance", configured))
}

/// Builds the lease proof the coordinator fences against for one authenticated request.
///
/// Every component is taken from gateway configuration, never from the request body. The lease
/// epoch is the configured numeric epoch, so a rotated deployment epoch changes the proof.
pub(super) fn lease_proof(
    instance: &str,
    caller: &str,
    session: &str,
    lease: &str,
    lease_epoch: u64,
) -> LeaseProof {
    LeaseProof::new(
        instance_id(instance),
        CallerId::new(digest("caller", caller)),
        SessionId::new(digest("session", session)),
        LeaseId::new(digest("lease", lease)),
        LeaseEpoch::new(lease_epoch),
    )
}

fn digest(domain: &str, value: &str) -> u64 {
    let mut input = Vec::with_capacity(domain.len() + value.len() + 1);
    input.extend_from_slice(domain.as_bytes());
    input.push(0);
    input.extend_from_slice(value.as_bytes());
    let hex = sha256_hex(&input);
    let mut bytes = [0u8; 8];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap_or(0);
    }
    // The durable SQLite store keys records by `i64`, so an identifier above the signed range
    // cannot be persisted at all. Masking to 63 bits keeps every derived identity storable, and
    // the low bit is forced on so no identity collides with the reserved zero value.
    (u64::from_be_bytes(bytes) & 0x7fff_ffff_ffff_ffff) | 1
}
