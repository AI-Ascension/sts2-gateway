# ADR 0022: Gateway-local co-op-native v1 peer-route binding

## Status

Accepted for the gateway component boundary on 2026-09-11. This is a safety
correction to the v1 consumer. It changes neither the `coop-native-v1` schema,
its copied artifacts, nor its digest.

## Context

The v1 envelope identifies an actor peer and an operation, but an envelope
field is supplied by the caller. The existing gateway bearer and lease checks
could therefore authenticate a local route without proving that its caller was
the configured local producer for that peer. Protocol ADR 0035 requires the
gateway to bind that route before a future game-mod-owned native message carrier
can use the existing operation identity.

An action fingerprint, actor string, generation, or matching observation is
not causal evidence of a native host settlement. The gateway must never use any
of those values to select a pending operation.

## Decision

When native v1 routes are enabled, the gateway operator configures exactly one
`STS2_COOP_NATIVE_PEER_TOKEN` and `STS2_COOP_NATIVE_PEER_ID` pair. Both values
are required together. Every native v1 route
requires the `x-sts2-peer-token` header. The token is checked against the
configured binding, while `check_lease` continues to authenticate the bearer,
caller, MCP session, instance, session, lease, epoch, and correlation.

The two values occupy separate namespaces. `STS2_COOP_NATIVE_PEER_TOKEN` and
`x-sts2-peer-token` are an opaque route-authentication credential; they are
never serialized into a v1 envelope, returned by the gateway, logged, or
forwarded to the mod. `STS2_COOP_NATIVE_PEER_ID` is the canonical `peer:…`
identity. It is compared only with the envelope's `actor_peer` where that
field exists, and every valid response observation must contain exactly one
`role: local` peer whose protocol-owned `peer_token` equals that identity.
Thus the configured canonical peer ID, scheduled request actor where present,
and returned local observation peer are the same identity namespace. The
gateway never compares either canonical identity with the credential. Harness
and MCP receive or carry only the canonical peer identity, never the
credential.

The private credential accepts the gateway's private identity policy: one to
128 ASCII alphanumeric, `-`, `_`, `.`, `:`, or `/` bytes, with no `..`.
The canonical peer ID instead follows the frozen v1 `peer_identity` grammar
exactly: `peer:` followed by 5 through 507 ASCII `[A-Za-z0-9_.:/-]` bytes
(512 bytes total). In particular, `..` is permitted in that protocol identity.

The binding captures its configured instance, session, lease ID, and epoch. A
route whose live fence no longer equals that tuple fails closed; a recovery
lease change requires a newly configured local producer binding rather than
silently carrying the old peer authorization forward. Legal-catalog, action,
vote, and rejoin envelopes must name the bound peer exactly. `recover` has no
caller-selected actor; it is admitted only through the retained binding.

There is one in-memory pending record per gateway local producer. An action,
vote, or rejoin stores its original route, operation ID, complete route
binding, and returned host authority fence before forwarding. A second pending
operation, including an exact duplicate, is rejected before downstream I/O.
Recovery is forwarded only for that same operation and binding. A return is
accepted only when its operation matches the retained record and, for an
initial route, its route matches too. Once an accepted or unknown return has
recorded host authority ID/epoch, a changed authority cannot settle that record
and is rejected. Terminal settled or rejected returns remove the pending record.

This component state is intentionally non-durable and bounded. A restart loses
it, so a caller receives an explicit unknown/recovery outcome rather than a
fabricated settlement or a replayed mutation. Game-mod remains responsible for
native sender authentication, carrier serialization, host legality, mutation,
and the host-issued witness. No native message, fingerprint, or internal host
identifier crosses into this gateway contract.

## Rejection and verification

The deterministic service tests cover absent/wrong peer authorization,
peer-substitution rejection, stale lease fencing, duplicate admission,
same-operation recovery, mismatched-operation and changed-binding recovery
rejection, and changed-authority refusal using loopback fakes.
They also reject an observation, catalog, action, or recovery response whose
sole returned local peer differs from the configured canonical peer, before
returning the response or clearing a pending operation. This response-ownership
check does not select a pending operation: that selection remains exact original
operation ID, original route, binding, and authority fence.
They demonstrate only gateway source/component behavior. A live native two-peer
session, first-party message serialization, host return delivery, native
mutation, and settlement remain unverified.
