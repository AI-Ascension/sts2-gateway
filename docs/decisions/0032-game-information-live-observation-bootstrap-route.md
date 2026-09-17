# ADR 0032: game-information live-observation bootstrap route

The protocol `game-information-live-observation-bootstrap-v1` profile carries the
first trusted parent observation and bounded visible entity references needed before
a live query can select an instance. Gateway owns the authenticated fixed route,
instance and lease fence, Harness lookup-binding scope fence, transport bounds, and
response validation. The game-mod remains the authority for native snapshot and
occurrence identity; Gateway never fabricates those references.

The public route is:

```text
POST /v1/instances/{instance_id}/game-information/live-observation-bootstrap
```

It forwards only to:

```text
POST /api/v1/game-information/live-observation-bootstrap
```

The request must be JSON, use the pinned schema digest
`6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2`, and carry a
current lookup-binding witness for the same instance, run, content manifest, locale,
and Harness authority epoch. Gateway checks the correlation and all identity headers
before forwarding. Responses are schema-validated and must echo scope and selector,
never expand requested limits, attest every visible entity to the parent snapshot,
and remain within Gateway's 16 KiB request and 128 KiB response bounds. A typed
`not_observable` producer response remains an explicit native-unavailable result with
null observation payloads.

Negotiated capabilities advertise
`game_information.live_observation_bootstrap` only when the operator enables the
explicit `STS2_GAME_INFORMATION_LIVE_BOOTSTRAP_ENABLED=true` deployment setting and
a current lookup-binding witness exists. The setting asserts that the pinned
producer handler is installed; it does not assert native snapshot availability.
Transport or contract-validation failure withdraws the per-instance support witness
until the bound owner authority changes. Existing query-v1,
lookup-binding-v1, and all other routes and bytes are unchanged.

This is an additive Gateway source/component contract. Native Mod production,
MCP startup composition, Harness use, and deployment/release compatibility remain
separate evidence.
