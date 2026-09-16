# ADR 0028: Continuation owner read and claim

## Status

Accepted for the gateway component contract. This is an additive, privileged
control surface; it does not implement native checkpoint restore or branch
execution.

## Context

Harness continuation may resume a selected branch only while the destination's
gateway lease, host fence, session, and installed host binding are current.
Persisted lease rows and historical operation records do not prove that a
live gateway process still owns the destination. A lost claim response must
be reconcilable without assigning the same lease epoch to a sibling operation.

Gateway owns current instance/session/lease ownership and fence admission.
Harness owns branch ancestry and execution attempts. Game-mod owns native state
and effects. No component may treat a historical gateway claim as evidence that
the process or native destination remains live.

## Decision

Add the versioned `sts2-continuation-owner-v1` frame contract and three fixed
routes:

| Route | `x-sts2-recovery-capability` and frame capability | Operation |
| --- | --- | --- |
| `POST /v1/recovery/continuation/owner/read` | `continuation_owner_read` | Read current owner state and non-secret fence identity. |
| `POST /v1/recovery/continuation/owner/claim` | `continuation_owner_claim` | Compare the exact current owner and durably claim its lease epoch for one operation ID. |
| `POST /v1/recovery/continuation/owner/lookup` | `continuation_owner_lookup` | Read a historical claim and current owner state separately. |

Frames are closed strict JSON and at most 16 KiB. The configured recovery
bearer, per-route capability header, frame capability, and configured harness
principal must all match. Responses omit lease tokens, host proof, paths, and
checkpoint material. Owner identity includes deployment, instance,
incarnation, boot, authority generation, host-fence identity and generation,
lease identity and epoch, session, and expiry.

The owner read is `available` only when the recovery store reports the ready
authority, matching current host fence, active unexpired lease, and completed
host installation, and the process still holds the matching in-memory lease
and live deadline. Persisted state without matching live process ownership is
`unknown`. `absent`, `expired`, `revoked`, and `unknown` remain distinct.

Claim is a SQLite compare-and-set against the same current owner tuple. An
operation ID and destination lease epoch are each single-use. An exact retry
returns the existing claim; an identity mismatch or sibling claim conflicts.
Retention is bounded at 4096 rows and fails closed at capacity. Lookup reports
historical claim data without promoting it to current ownership. It never
retries an effect.

## Failure and compatibility behavior

Missing or unavailable authority returns an explicit unavailable error. A
stale tuple or competing claim conflicts before a continuation effect.
Oversized frames, invalid UTF-8/JSON, duplicate JSON keys, unknown fields,
wrong principal, missing capability, and malformed identities reject before
store mutation. Process restart with no confirmed live lease reports
`unknown`; a historical record cannot authorize action or replay.

The routes and contract are additive. Existing recovery-v1 and gameplay wire
formats are unchanged. Clients must use this contract explicitly; older
clients gain no continuation capability.

## Validation oracle

Production dispatcher tests use the real SQLite recovery store and check read,
claim, exact duplicate, sibling conflict, stale fence, historical lookup
after live ownership disappears, restart-to-unknown, and auth/frame rejection.
Library tests cover expiry, incomplete host installation, non-secret output,
and lease epoch uniqueness. Contract fixtures and pinned schema/digest verify
the closed frame format.

This is deterministic component evidence. It does not establish native restore,
process restart continuity, MCP mapping, exact-host behavior, or continuation
assurance.
