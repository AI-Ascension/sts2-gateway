# ADR 0019: candidate Runtime-v4 expert rest-action gateway route

- Status: Candidate transport implemented; protocol and downstream adoption unadmitted
- Date: 2026-09-08
- Owner: `sts2-gateway` for the HTTP assignment; `sts2-protocol` for the neutral contract
- Prospective consumers: `sts2-game-mod`, `sts2-mcp-server`, `sts2-harness`

## Requirement and context

The native rest-site options exposed by the Runtime-v4 expert observation need a separately pinned
action profile. The existing `runtime-v4-expert-action` profile is potion-only, so rest options,
selector follow-up actions, and their effect evidence must travel through a distinct additive
contract. The gateway assignment must preserve the existing authenticated instance/session/lease
boundary and fail closed on malformed, foreign, stale, oversized, or semantically inconsistent
messages.

The candidate contract is `runtime-v4-expert-rest-action-v1` with profile `expert-rest-action`,
artifact `sts2-protocol/runtime-v4-expert-rest-action`, and schema digest
`bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd`. Its copied manifest remains
`candidate` with no admitted consumers.

## Decision and ownership

Expose these fixed gateway paths:

```text
POST /v4/instances/{instance_id}/expert-rest-action
GET  /v4/instances/{instance_id}/expert-rest-actions/{operation_id}
```

The first route requires mutate scope, JSON content type, a bounded request body, and a complete
request envelope. The second requires read scope, an empty body, and a safe operation-id path
segment. The gateway forwards only to:

```text
POST /api/v4/runtime/expert-rest-action
GET  /api/v4/runtime/expert-rest-actions/{operation_id}
```

The gateway owns HTTP method/path admission, authentication and lease fencing, fixed downstream
paths, body/response limits, exact artifact identity, and response status/identity validation. The
game-mod owns native rest-site meaning, host IDs, host observation, effect authority, and native
framing. MCP owns tool/catalog mapping and the harness owns scheduling, model/provider calls,
episodes, replay, and scoring.

Settled responses require a valid nested `runtime-v4-expert` observation, a generation-fenced typed
transition, and an option-specific `rest-effect-witness-v1`; root and transition witnesses must be
equal. Selector responses require a complete typed legal-action catalog with consistent counts and
visible choices. The gateway retains a bounded selector-admission catalog across response
observations because a completed selector may return its final observation to `rest`. A selected
card or player must have appeared in the earlier valid catalog. Missing prior admission is a
response error and cannot be inferred from an ID prefix.

## Compatibility, rejection, and cancellation

This is a minor additive gateway/profile change. Existing Runtime-v1, Runtime-v2, Runtime-v3,
Runtime-v4 expert, map, co-op, and legacy routes retain their paths, artifacts, and behavior. The
candidate remains unadmitted until every prospective consumer pins the same digest and passes its
own serialized checks. Invalid content type, body, route, envelope, identity, lease, epoch,
correlation, schema, selector, generation, status, observation, transition, or witness data fails
closed before a response is returned. Downstream malformed or oversized responses return `502`;
downstream connection failures use the existing unavailable status path. A stale or inactive lease
is rejected before forwarding.

An accepted mutation is never retried because a caller times out or disconnects. Reconciliation is
read-only and uses the original operation identity. Selector admission is bounded and retained for
the service lifetime; a restart has no prior catalog and therefore fails closed for a completed
selector until a fresh selector-request response establishes context.

## Deterministic oracle and evidence boundary

The gateway tests validate all 16 candidate goldens for dispatch and reconciliation, reject all 22
schema-valid mutation fixtures, and validate both serialized Smith and Mend producer-shaped
lifecycles message by message. The service test checks the fixed downstream paths, bodyless
reconciliation, content type, and malformed profile rejection before downstream access. The artifact's
`SHA256SUMS` file validates the copied schema, conformance case, manifest, goldens, mutations, and
producer fixtures.

These checks establish source/component behavior only. They do not prove a managed native producer,
host legality or effect settlement, MCP/harness consumption, deployment, or release compatibility.
