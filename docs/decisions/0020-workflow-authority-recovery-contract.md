# ADR 0020: Runtime-v2 workflow authority and recovery contract

- Status: Accepted for the gateway-local T12 source/component boundary
- Date: 2026-09-10
- Owner: `sts2-gateway`
- Requirements: WF-039, WF-040, WF-041

## Context

Runtime-v2 already has gateway-owned instance, session, lease, epoch, fencing, bounded operation
ledger, and retained read-only receipt seams. Its frozen envelope does not carry a boot epoch, and
the attached executable does not establish durable boot-epoch issuance or host restart continuity.
Workflow admission therefore needs an owner-boundary contract that can reject stale ownership and
make unsupported recovery claims visible without adding another scheduler, store, protocol, MCP,
or host adapter.

## Decision

Add the gateway-local `RuntimeV2Authority` and `RuntimeV2RecoveryContract`. The authority binds
instance, session, lease, lease epoch, and an owner-issued boot epoch. A workflow ledger is
constructed with that authority and a `RuntimeV2RecoveryCapabilities` value that advertises
harness, gateway, host, and machine recovery independently. Unsupported domains return a typed
rejection instead of being inferred from another domain.

When a recovery contract is installed, mutation, state refresh, cancellation, and reconciliation
must use the corresponding authority-bearing ledger methods. The old methods reject with
`AuthorityRequired`, so a workflow caller cannot silently fall back to the legacy identity-only
entry point. Authority identity or lease mismatches reject before ledger replay or forwarding;
changed boot epochs return `StaleBootEpoch`. An old operation ID remains only an idempotency key and
does not authorize a fresh dispatch.

Receipt reconciliation first requires an available `RuntimeV2ReceiptRetention` claim. It then uses
the existing private receipt-request constructor and `RuntimeV2ForwardingPort::read_runtime_v2_receipt`;
that seam is read-only and cannot authorize or apply mutation. Persisted workflow state records its
boot epoch. A missing or changed epoch rejects restore, while legacy bindings retain compatibility
with state that has no boot epoch.

## Compatibility and rejection behavior

This is an additive gateway-local source/component contract. It changes no Runtime-v2 artifact
bytes, MCP route, protocol/mod file, attached executable route, or transport field. The existing
`RuntimeV2Ledger::new` constructor remains the legacy/component lane. Callers using a recovery
ledger must migrate to the authority-bearing methods and provide a fresh boot identity for a new
ownership context. A recovery ledger rejects implicit mutation/state/cancel/reconcile calls;
authority mismatch, stale boot, unsupported recovery domain, invalid retention bounds, unavailable
retention, and missing/mismatched persisted boot identity each have typed fail-closed outcomes.

The contract is source/component evidence only. It does not issue durable boot epochs, prove
process or host restart continuity, promote the attached executable to restart-safe, or establish
live game/mod compatibility.

## Deterministic oracle

`runtime_v2_authority.rs` verifies independent failure-domain capabilities, retention validation,
owner and boot fencing, implicit workflow rejection, duplicate replay without a second dispatch,
receipt gating before a read, read-only retained settlement, and restore rejection for missing or
changed boot identity. Existing Runtime-v2 ledger and persistence tests remain on the legacy
constructor and preserve its compatibility path.
