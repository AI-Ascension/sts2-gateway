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
