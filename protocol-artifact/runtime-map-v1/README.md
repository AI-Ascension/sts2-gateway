# runtime-map-v1

`runtime-map-v1` is the additive, host-owned read-only map visibility profile. It carries a
bounded graph of player-visible nodes and directed edges, explicit availability/completeness /
freshness labels, pre-start/current/unavailable position, visible history, and generation-bound
host action bindings.

The independent snapshot schema identity is `visible-map-v1`; `projection_version` identifies the
producer projection. `map_instance_id`, `act_id`, and `scope_id` are nullable when the host cannot
truthfully identify an available map. The profile never carries hidden map state, future outcomes,
RNG, host objects, or harness/provider identifiers.

Coordinates may overlap and visible components may be disconnected; those are preserved as
host-provided projection facts. The protocol does not infer missing graph relations or fabricate a
current-to-destination edge.

Each binding preserves three identities independently: the stable projection `graph_node_id`, the
exact host `host_action_id`, and the opaque serialized `select_map_node` action-option `node_id`.
The option ID is bounded and unique within the binding set, while it does not have to equal a graph
node ID.

The checked-in fixture uses synthetic game/mod version strings for deterministic conformance; it
does not establish licensed host compatibility or a current game installation.

Schema validation is structural. Consumers must also enforce the exact schema digest/provenance,
semantic graph invariants, and UTF-8 byte bounds: 128 bytes for build/version text and 256 bytes
for reason text. Both fields exclude C0, DEL, and C1 controls. Schema `maxLength` counts Unicode
characters and does not replace the mandatory byte check; valid multibyte text remains supported.

Canonical message bytes are compact UTF-8 JSON with sorted object keys and normalized set-like
collections. `map_snapshot_digest`, `map_content_digest`, `map_topology_digest`, and
`map_navigation_digest` have separate meanings; none grants mutation authority.
