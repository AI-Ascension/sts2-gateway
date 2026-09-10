# ADR 0018: allocation-response failure closes admission

Status: implementation candidate; independent review pending.

## Requirement and ownership

G62 implements watchdog invariants 1, 2, 11 and 14 at the gateway allocation
boundary. A response may publish recovery authority only for the exact current,
durably valid lease with an installed host binding and matching durable fence.
Both its monotonic deadline and local admission state must still permit use.
The gateway owns this check; the host remains the final mutation authority.

## Failure and cancellation

Failure after acquisition immediately closes local admission. Attempt durable
gateway revocation and bounded host revocation using the retained lease identity,
even if local admission was already closed. Never treat a failed storage write
or host acknowledgment as successful cleanup. Retain unresolved host revocation
for authenticated reconciliation. Fresh allocation and new mutation remain
blocked while cleanup is unconfirmed. Successful durable host cleanup permits a
fresh epoch only if operator stop/revocation did not already close the context.
Historical non-mutating recovery remains separate from new mutation admission.

Cleanup provenance is an in-memory marker bound to the exact unreturned lease ID.
It survives a failed cleanup attempt, not a gateway restart. A matching durable
host revoke ACK may clear quarantine only for that marker and only without a
shutdown request. Explicit operator revocation, ordinary release, and shutdown
remove that permission before their fallible cleanup. A marker for another lease
cannot authorize a fresh allocation. This does not replace watchdog-owned durable
operator intent across process or machine restart.

All validated external recovery revocations cancel cleanup permission, including
the `shutdown` reason. Automatic cleanup retries instead occur on a subsequent
allocation request only after its configured instance/caller/session identities
match. Each request attempts at most one retained exact-lease revocation; no new
allocation occurs unless its host ACK commits durably. Malformed or mismatched
allocation requests cannot trigger cleanup, and explicit shutdown never retries.

Installation activation failure after a durable host ACK also enters cleanup.
Readiness on ordinary and recovery mutation paths verifies the full canonical
grant digest and current durable fence, not merely an `INSTALLED` state label.

## Compatibility and oracle

This is a fail-closed correction to the unpublished recovery allocation path.
No route, request field, frozen contract artifact or schema digest changes.
Successful responses and non-recovery allocation remain unchanged. Existing
errors describe rejection; no lease token is returned on failure.

Deterministic tests require rejection of a substituted lease, inactive or expired
authority, and fence mismatch. A real SQLite competing write transaction tests
revocation failure: after the writer releases its lock, the still-active durable
row must not bypass local quarantine. A bounded synthetic authenticated host
tests failure after installation. Gateway revocation/pending state is not proof
that the host has durably revoked; unavailable host cleanup remains unverified.

A test-only, thread-local expiry fault runs after successful durable installation
commit and before activation. Its consumption is asserted, so pre-commit expiry
cannot satisfy that regression. A separately corrupted SQLite grant digest must
deny ordinary and sideband readiness before allocation-response cleanup runs.
Signed delayed revoke acknowledgments test all stop and marker cases above.
