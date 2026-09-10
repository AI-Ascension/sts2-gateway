# ADR 0018: proposed co-op retained receipt query route

- Status: Proposed and unadmitted
- Date: 2026-09-08
- Owner: `sts2-gateway`
- Prospective consumers: `sts2-game-mod`, `sts2-mcp-server`, `sts2-harness`

## Context

Recovery clients need to ask the authoritative co-op host whether one already-admitted action has
a retained receipt after a timeout or disconnect. A receipt lookup must preserve the original
operation lineage and must not turn recovery into a new observation, reconciliation, queue entry,
or mutation. The gateway owns the transport, authentication, identity, lease, epoch, correlation,
body, response, and fixed-route boundaries. The game-mod owns native ID resolution, host authority,
cache authenticity, and native packet framing. MCP and the harness consume the neutral result.

The proposed language-neutral profile is `sts2-protocol/coop-receipt-query-v1`, revision 1, with
schema digest `3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c`. Its manifest is
intentionally `proposed_unadmitted` with an empty `consumers` array. The existing managed JSON
profile and the mod-owned native binary query artifact remain separate contracts.

## Decision

Expose exactly:

```text
POST /v1/instances/{instance_id}/coop/receipt-query
```

The route requires the normal read scope and the existing authenticated active lease context. It
accepts only canonical compact UTF-8 JSON with the frozen top-level and nested member order,
single-line terminator, exact provenance and digest, closed members, bounded identities and
generations, sorted distinct participants containing the actor, and equal expected/before
generations. The gateway forwards only the validated bytes to:

```text
POST /api/v1/coop/native/receipt-query
```

The gateway validates the response against the same profile and request identity. Responses must
be explicitly scoped as `retained_receipt`; settled receipts must advance generation and agree
with action/effect identity; accepted and rejected receipts must carry matching receipt status;
unknown and recovery-required responses carry no receipt. A downstream response that is malformed,
foreign, semantically inconsistent, or over the response limit fails with `502`. Missing or
invalid request content fails before downstream access, and stale or inactive lease context fails
at the existing lease fence.

This route is read-only. The gateway performs no fresh observation, reconcile, queue admission,
retry, host-state synthesis, or mutation authorization. The adapter boundary may resolve opaque
IDs and use native framing only in the game-mod owner.

## Compatibility and cancellation

This is an additive proposed profile and route. Existing Runtime-v1, Runtime-v2, Runtime-v3,
Runtime-v4, map, and coordinator-report surfaces retain their routes, bytes, and limits. A caller
timeout or disconnect does not cause the gateway to retry a receipt query; a later query repeats the
same operation identity. Lease release, shutdown, stale epoch, wrong instance, or wrong session
fences the request before forwarding. The route does not admit a new operation, so there is no
gateway cancellation record to synthesize.

## Deterministic oracle

`service_receipt_query_tests.rs` validates every frozen status golden and rejects malformed or
semantic vectors for participant membership/order, duplicate keys, numeric spelling, generation
lineage, evidence scope, response status/receipt agreement, settled generation advancement,
action/effect agreement, and repeated identity. The route test checks JSON content type and active
lease admission before any downstream connection. The copied artifact's `SHA256SUMS` verifies the
schema, conformance case, manifest, goldens, and invalid fixtures.

These tests establish deterministic gateway source/component behavior. They do not admit the
profile, prove a native host producer, prove cache authenticity, or establish live multiplayer,
MCP, harness, deployment, or release compatibility.
