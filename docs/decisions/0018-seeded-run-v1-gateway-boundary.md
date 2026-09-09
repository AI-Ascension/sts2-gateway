# ADR 0018: Seeded-run v1 gateway boundary

- Status: Accepted for the source and component gateway lane; native host settlement remains unverified
- Date: 2026-09-09

## Context

An explicit seeded launch needs a complete native selection context, a lease-fenced operation
identity, and evidence that the host actually started the requested run. Admission by the gateway
does not establish that the host launched anything. A caller timeout can also leave a native start
in flight, so retrying the mutation would risk two runs.

The gateway must expose one fixed public route pair, forward only the corresponding fixed native
routes, retain an operation across an uncertain response, and distinguish transport correlation
from the semantic identity of the launch. The existing Runtime-v2 journal is the configured
durability boundary and must retain its format and digest.

## Decision

The attached service owns these instance-scoped routes:

- `POST /v2/instances/{instance_id}/seeded-run` with the mutate scope.
- `GET /v2/instances/{instance_id}/seeded-operations/{operation_id}` with the read scope.

The adapter forwards only `POST /v2/seeded-run` and
`GET /v2/seeded-operations/{operation_id}` to the native loopback endpoint. Every message is
checked against the copied `seeded-run-v1` metadata, selected-context digest, caller identity,
session, lease, epoch, correlation, operation, and bounded schema before it crosses the boundary.
The game-mod owns seed interpretation, native selection, profile/save behavior, canonical seed
readback, and the settlement witness. The gateway owns admission, fencing, idempotency, bounded
retention, and transport outcome mapping.

The operation digest serializes the validated start request with `correlation_id` cleared.
Correlation is a transport-attempt identity and may change when an MCP retry receives a new JSON-RPC
id; operation identity, seed, mode, context, lease, and generation remain semantic inputs. A
semantic duplicate replays the retained result without another native POST and binds the replayed
response to the retry's correlation. Reusing an operation with another semantic request is rejected
before forwarding. A stale identity or lease is rejected before duplicate lookup can authorize it.

An accepted or unknown result is reconciled only through the fixed native GET receipt route. The
GET decoder validates the response envelope, operation, correlation, and lease lineage; the ledger
then validates the full receipt against the retained original start request. Reconciliation may
advance an accepted or unknown operation to settled, but never dispatches a second start. A settled
response retains the host's original start generation; the reconciliation request's generation is
not copied over it.

When a new gateway ledger has no retained operations, the caller's fenced generation initializes
the local binding so a nonzero authoritative host generation is not rejected as an uninitialized
zero. Once an operation is retained, the binding is exact and later requests must match it. The
native response remains authoritative for settlement and successor generation; caller input alone
does not create settlement evidence.

If `STS2_RUNTIME_V2_JOURNAL` is configured, seeded markers are written to its separate
`*.seeded-run.json` sidecar. Admission is checkpointed before dispatch and again after the result.
On restart, an admitted operation without a durable result and an accepted result are converted to
`unknown` with no mutation replay; reconciliation performs a read-only receipt lookup. The sidecar
uses the existing bounded atomic journal writer and lock. Without the environment variable, the
seeded ledger is in memory.

## Compatibility and limits

This is an additive gateway and protocol-artifact consumer. Runtime-v1, Runtime-v2, and the
existing Runtime-v3 artifact are unchanged. Rejection, timeout, malformed-response, capacity, and
lease-fence behavior is gateway-owned and typed; cancellation does not undo a dispatched native
start. The source/component tests prove fixed routing, schema validation, semantic duplicate
replay, accepted/unknown read-only reconciliation, and sidecar restore without redispatch.

The sidecar is bounded owner-managed process storage, not a distributed registry, transaction log, or
proof that a crashed native process settled. A restart can recover only the gateway marker and then
requires native receipt evidence. The loopback route and selected-context contract do not prove a
real STS2 installation, host compatibility, save isolation, provider execution, visible gameplay,
or release readiness; those require host/game-mod evidence from their owning lane.

## Deterministic oracles

The component gate must show that an exact semantic duplicate with a new correlation performs one
native POST and returns a schema-valid replay, a conflicting reuse performs zero additional POSTs,
accepted and unknown results use one read-only GET to settle, a recovered sidecar operation uses no
POST after restart, and mismatched metadata, context, operation, identity, generation, or lease is
rejected before forwarding. These are component oracles and do not substitute for live host proof.
