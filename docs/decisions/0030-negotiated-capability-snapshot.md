# ADR 0030: authenticated negotiated-capability snapshot

## Decision

The Gateway owns `GET /v1/instances/{instance_id}/negotiated-capabilities`.
It requires normal bearer read authorization, an empty body, the current
instance/session/MCP-session/lease/epoch headers, and a valid correlation id.

The response is bounded to 16 KiB and repeats those identities with the
authenticated caller’s granted `read`, `mutate`, and `control` bits. It includes
only known game-information offers whose producer capabilities have been
validated and are bound to the current producer authority. On a cold or
authority-stale cache, it first performs that same authenticated, lease-fenced
validated capabilities exchange; MCP startup therefore has no undocumented
prewarm step.

## Rejection and compatibility

An unavailable or invalid producer capabilities exchange is surfaced with its
bounded typed error. Lease fencing is rejected before a producer call. Unknown
producer query kinds are omitted. The fixed Gateway producer profile is source
identity only; MCP maps these Gateway operation/revision values to its own tool
revisions and does not infer a `*-mcp` profile name. A successful pinned lookup-binding discovery is retained only as a
Gateway-local witness tied to the current producer authority; its two fixed
operations are advertised only while that witness remains current. The witness
commits the harness-owned scope and authority epoch through its canonical
binding id, without exporting a scope or turning it into a grant. A separate
nullable Runtime-v3 baseline witness is emitted only after a current,
authenticated and lease-fenced state exchange validates the pinned response
identity, provenance and envelope. It proves the configured state adapter and
pinned profile, not that every runtime-v3 operation is native-certified. Its
fixed recovery catalog is only `reobserve` and `reconcile`; `release_lease` and
`stop_episode` are omitted. This is additive: existing routes and capability discovery remain
unchanged.

## Limits

An offer has two independent limit groups. `wire_limits` measures the exact
serialized Gateway-to-producer request and producer response bodies; a
bodyless operation has a zero request limit. `content_limits` measures MCP's
returned semantic content bytes and page items. MCP owns the separate
descriptor limit for serialized `{name, arguments}` and applies each Gateway
wire limit only after mapping, without inferred wrapper overhead. The artifact's
`limit-examples.json` makes these units concrete for capabilities, state,
lookup-binding, and query calls.

## Ownership

The response is a Gateway boundary artifact, even though MCP consumes it. It
does not move MCP catalog policy, game semantics, or producer authority to a
shared contract.
