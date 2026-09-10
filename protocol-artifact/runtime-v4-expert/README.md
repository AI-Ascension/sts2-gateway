# `runtime-v4-expert` additive fair-play profile

This profile extends the frozen `runtime-v3-gameplay` boundary with ordinary player-visible
context needed for strategic decisions: run character/location, block, powers/statuses, relics,
potions and capacity, richer card metadata, enemy status and rendered target context, a visible map
graph, contextual choices, and parameterized potion/rest/selection actions.

The profile is independently versioned. A Runtime-v3 consumer must not parse this profile as
Runtime-v3, and a consumer must reject a different schema digest or provenance. Every field is
required at the profile boundary. A nullable value means that the value was unavailable or not
visible in the current host surface; an empty array means the surface was observed and empty.
Unknown fields, unknown enum values, hidden values, unrevealed map content, draw order, future RNG,
and private host data fail closed and have no representation.

The schema and this artifact are inert protocol evidence. The mod remains the authority for host
extraction and mutation, the gateway owns lifecycle and routing, MCP owns tool mapping, and the
harness owns coordination and replay. No schema or conformance fixture proves target-build
compatibility or live gameplay.
