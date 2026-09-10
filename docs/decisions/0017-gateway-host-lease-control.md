# ADR 0017: gateway-issued host lease-control consumer

Status: accepted for the gateway recovery/control boundary.

## Context

The gateway owns lease issuance, but the managed host must durably install the
exact lease authority before gameplay or recovery operations can be admitted.
The existing recovery-v1 lease-acquire path establishes gateway authority and
cannot be forwarded to a host: doing so would allow a second issuer. The host
lease-control sideband therefore carries the gateway-issued grant, its digest,
and a bounded acknowledgment for installation, renewal, and revocation.

## Decision

Expose the host lease-control operations through the existing authenticated,
fixed loopback recovery transport. The gateway validates a duplicate-free,
closed frame, contract/schema identity, request kind, gateway actor, grant
lineage, and HMAC proof before forwarding. Only the three fixed operation kinds
are accepted; arbitrary paths, headers, or bodies are not exposed.

The gateway persists a protected grant and `PENDING_HOST_INSTALL` state before
the first send. A matching durable `INSTALLED` or duplicate acknowledgment is
required before admission. Renewal and revocation use the same installation
identity and grant lineage, persist the resulting host acknowledgment, and
fail closed on transport uncertainty. A timeout or disconnect never mints a
replacement lease and remains reconcilable by the original operation identity.

A durable `INSTALLED` row is historical across a gateway process restart, not
an authorization to resume mutation. Runtime admission and idempotent install
replay additionally require the exact current-process grant cache to match the
row's installation identity, digest, and reconstructed boot/fence/lease grant.
A missing or mismatched cache fails closed while leaving the protected binding
readable for historical inspection; only a fresh host acknowledgment can
establish a new process authority.

The persisted representation stores the grant digest and a fence-token digest,
not the plaintext fence token. A plaintext token may remain only in the
bounded in-memory retry copy for the current process lifetime. Host install
generation is distinct from the gateway lease epoch and must match on every
acknowledgment.

## Compatibility and ownership

This is an additive `watchdog-host-lease-control-v1` control-plane contract;
the frozen recovery-v1 and runtime-v3 gameplay artifacts are unchanged.
Gateway owns issuance, identity, lease/epoch checks, persistence, fixed
routing, authentication, and admission gating. The managed host owns durable
installation and host-side acknowledgment. A passing gateway test or HTTP
response does not establish live host settlement.

## Rejection oracle

Reject oversized, malformed, duplicate-member, unknown-field, invalid-UTF-8,
wrong-contract, wrong-schema, unsupported-kind, expired, mismatched-lineage,
invalid-proof, stale-sequence, and invalid-configuration frames before a
state mutation. Reject acknowledgments whose installation, grant, boot, fence,
lease, or host-install-generation identity does not match the pending gateway
operation. Transport uncertainty remains an explicit pending/unknown outcome
for reconciliation.
