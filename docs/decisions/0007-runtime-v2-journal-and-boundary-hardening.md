# ADR 0007: Runtime-v2 journal and boundary hardening

- Status: Accepted for the attached Runtime-v2 component lane; host/system durability remains unverified
- Date: 2026-09-02

## Context

The Runtime-v2 operation ledger must preserve the truth of an admitted mutation when the caller or
gateway process stops at an uncertain point. An in-memory receipt cannot distinguish a settled
operation from work that was lost during restart. The attached runtime also terminates a bearer
credential and must reject malformed or cross-instance requests before any downstream forwarding.

The first implementation remains a single configured instance. It must therefore establish bounded
operation retention and explicit failure behavior without implying a multi-instance supervisor,
production database, token rotation service, or host compatibility.

## Decision

The attached runtime may use `STS2_RUNTIME_V2_JOURNAL` to enable a version-1, bounded JSON journal.
The journal stores the binding observation and retained operation request/result records. Writes use
a temporary sibling file, flush it, and replace the configured path. The runtime acquires an
exclusive operating-system journal lock on a stable sibling path for the service lifetime; startup
fails closed when another process already owns that journal. Each instance must therefore receive a
distinct configured journal path. After replacement, the adapter syncs the parent directory on
platforms that support directory synchronization. An admitted operation is checkpointed before
mutation dispatch and again after the downstream result. If the first checkpoint fails, the
operation is removed and no mutation is dispatched. If the terminal checkpoint fails, the operation
remains retained as `unknown` with `sts2.runtime/persistence_uncertain`; it is never blindly resent.
On restart, missing or `accepted` results become `unknown` with `sts2.runtime/restart_uncertain`,
while valid settled receipts replay without contacting the mod.
Identity, lease epoch, request digest, message kind, and bounded result shape are revalidated before
state is restored; a mismatch rejects the journal rather than adopting another instance's state.

The journal is an owner-managed component adapter, not a claim of production storage durability or
a supervisor. The current attached process still owns one configured instance and one sequential
request loop. The lock prevents accidental same-path multi-process ownership but does not provide a
registry, lease service, cross-host storage guarantee, or recovery of a crashed downstream process.
`STS2_RUNTIME_V2_OPERATION_CAPACITY` is an explicit retained-operation bound from 1 through 64;
capacity exhaustion returns a typed overload response. This bound is not a global queue, fairness
policy, or four-instance isolation implementation.

Gateway bearer authentication compares the complete `Bearer <token>` value with a
length-independent byte accumulator. The caller token is terminated at the gateway; the separate
configured mod token is emitted only on the fixed downstream routes. Header, method, path, identity,
lease, epoch, correlation, and body checks remain required before forwarding.

## Compatibility

The journal environment variable and bounded capacity setting are additive process configuration;
the lock sibling and path-ownership rule are also process-boundary behavior; none changes the frozen
Runtime-v2 wire artifact or route names. Exact duplicate action requests compare the canonical
operation request with transport correlation excluded, replay their retained result before
generation revalidation, and rebind the response correlation to the retry. Conflicting operation
reuse remains `idempotency_conflict`. Runtime-v1 behavior is unchanged. Restart, credential
rotation, global backpressure, four-instance isolation, live mod compatibility, and host settlement
require separate evidence.

## Deterministic oracles

The gateway tests must show failed admission persistence causes zero dispatches, failed terminal
persistence yields one application and an `unknown` retained result, exact replay causes no second
dispatch even after the authoritative generation advances, malformed or mismatched journal state is
rejected, a second process cannot acquire an in-use journal lock, missing/incorrect bearer values
fail before lease processing, and operation capacity rejects work above its configured bound. These
tests are component evidence only.
