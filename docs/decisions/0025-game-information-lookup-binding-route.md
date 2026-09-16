# ADR 0025: game-information lookup-binding route

Accepted for the bounded LBR v1 gateway transport; producer and live host behavior remain
unverified.

## Requirement and ownership

The Runtime-v3 harness needs an authenticated owner path for lookup-binding discovery and
observation. Gateway owns the fixed route, caller authentication, instance and current lease
fencing, locale forwarding, bounded transport, and validation of the returned protocol envelope.
Harness owns the request scope and `authority_epoch`; game-mod owns `game_profile` and
`content_manifest_id`; the protocol artifact defines the canonical binding identity. Gateway does
not create or retain game observations.

The copied `game-information-lookup-binding-v1` artifact is pinned at schema digest
`f10f9af01d6be1de104069ba842e7971971e88f27553e782e81174ee7aa1cd58` and protocol source commit
`843fddbd3b5875d01d7f99d3433f872b0a4d7681`. Its schema inventory is verified by
`protocol-artifact/game-information-lookup-binding-v1/SHA256SUMS`. Contract changes require a
new reviewed pin.

## Fixed route and validation

The public route is:

```text
POST /v1/instances/{instance_id}/game-information/lookup-binding
```

Its closed request contains only `operation`, `project_id`, `run_id`, `episode_id`, `agent_id`,
and `authority_epoch`. The operation is `discovery` or `observe`. Existing authentication and
lease checks run before forwarding. The gateway sends the fixed producer path
`/api/v1/game-information/lookup-binding`, forwarding the active instance, caller, session, lease,
lease epoch, correlation, and configured locale headers. The harness-owned `authority_epoch` is
checked against the request and response binding; it is a separate identity from the gateway's
lease epoch.

The response must be duplicate-free JSON within 256 KiB and validate against the pinned schema.
Gateway also checks the schema digest, correlation, operation and HTTP status pairing, request
scope, authenticated instance, configured locale, required protocol capability, observation
binding, and the artifact-defined canonical binding ID. That ID is SHA-256 over compact UTF-8 JSON
with lexicographically sorted members, using the request scope and authority epoch plus the
producer-declared game profile, content manifest, and locale. `instance_id` is checked separately
and is not part of the digest.

Malformed, duplicate, oversized, foreign, schema-invalid, or status-inconsistent producer
responses return a bounded 502 gateway error. A valid typed `error_response` keeps its producer
status and body when the status is 4xx or 5xx. Invalid requests return 400. Authentication and
stale lease failures return before any producer connection. Cancellation and transport failures
use the existing bounded game-information transport outcomes.

## Compatibility and deterministic oracle

This is an additive gateway route. Existing routes, schemas, and artifacts are unchanged. The
deterministic oracle invokes the production dispatcher with a synthetic loopback producer and
checks discovery and observation success, exact fixed path and forwarded identity headers,
malformed and duplicate JSON, foreign instance, oversized response, inconsistent status, and
authentication and stale-lease rejection before forwarding. The checked-in protocol goldens and
checksum inventory establish copied-artifact integrity and source/component behavior only; native
extraction, host compatibility, MCP registration, live harness execution, deployment, and release
compatibility remain unverified.
