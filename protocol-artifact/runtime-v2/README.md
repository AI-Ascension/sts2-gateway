# `runtime-v2` release-like artifact

This artifact defines one bounded gameplay-operation profile: the `end_turn` action in a
`combat/player_turn` observation. It carries explicit admission, settlement, rejection, uncertainty,
cancellation, and operation reconciliation without owning transport, host objects, leases, or game
authority.

The normative source is [`schemas/runtime-v2.schema.json`](../../schemas/runtime-v2.schema.json),
and `schema.json` is its byte-identical package copy. The manifest digest identifies the exact schema
source bytes. `SHA256SUMS` covers the source, package, manifest, goldens, and conformance case.

The goldens are deterministic, sanitized vectors for the frozen contract. They do not prove that a
consumer, host, gateway, MCP server, harness, or live gameplay action is compatible.
