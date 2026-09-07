# ADR 0017: Runtime-map read-only snapshot route

- Status: Accepted for the additive `runtime-map-v1` profile
- Date: 2026-09-07
- Owner: `sts2-gateway`
- Consumers: `sts2-mcp-server`, `sts2-harness`, `ascension-map-visualizer`, and the
  `sts2-game-mod` host boundary

## Context

The map visualizer needs a bounded, generation-fenced view of the visible campaign graph. The
gateway must provide a fixed transport boundary while the game-mod remains the owner of host
observation and map meaning. The existing Runtime-v1, Runtime-v2, Runtime-v3 gameplay, and
coordinator-report routes must keep their catalogs, artifact bytes, limits, and authority rules.

The neutral contract is `sts2-protocol/runtime-map-v1`, finalized at protocol commit
`d9ffb190ad8990e15f43d7992581dcb2d60b1971`. Its copied schema digest is
`6340f3cbe6c1b5728144fe89fdfdf8645acf2f59a77c0e0c30ebfeafc77515d8`.

## Decision

The attached gateway exposes exactly one additive route:

```text
GET /v1/instances/{instance_id}/map-snapshot
```

The request is bodyless and passes the existing lease, caller, MCP-session, instance, session,
lease, epoch, and correlation fences before any downstream connection. The gateway forwards only
`GET /api/map/v1/snapshot` to the configured game-mod listener. It does not accept a caller-chosen
path, method, target, or map mutation.

The gateway consumes the copied `runtime-map-v1` schema and checks the response provenance,
protocol version, schema digest, response kind, configured identity headers, lease epoch, root and
snapshot generations, and the bounded response size of 256 KiB. It also checks the visible graph's
bounded node, edge, and binding collections, unique node IDs, valid coordinate bounds, known edge
endpoints, duplicate-edge rejection, acyclicity, valid visited position/history and terminal
references, and generation-bound navigation bindings. Overlapping coordinates and disconnected
visible components are preserved because they are projection facts. Each binding keeps the stable
graph node ID, host action ID, and opaque serialized action-option ID independent; action-option IDs
are bounded and unique. The downstream projection is returned only after those checks pass.

This route is read-only. A downstream failure, malformed response, stale or foreign identity,
unknown field, schema mismatch, semantic graph failure, or response over the limit fails closed as
an HTTP error; the gateway does not substitute a different instance, retry a map request, or
invent an unavailable snapshot. The existing five-second bounded HTTP exchange and header/body
limits remain in force.

The route is an additive compatibility change. Legacy profiles and their copied artifacts remain
unchanged. The protocol artifact is inert copied release-like data; no protocol implementation
crate is added as a gateway dependency. Host compatibility, live map observation, and visualizer
rendering remain separate evidence claims.

## Deterministic oracle

`runtime_map_forwarder_tests.rs` accepts the current response golden and rejects wrong identity or
generation, unknown fields, duplicate IDs, duplicate edges, cycles, invalid action options and
bindings, and an oversized node collection before forwarding. The service route tests cover the
bodyless fixed route, lease admission, downstream path, response budget, and typed failure path.
The copied manifest, schema, conformance case, and three goldens are checksum-verified in the
consumer worktree.

These tests establish source and synthetic component behavior. They do not prove a live game host,
game-mod map projection, map freshness, or visualizer rendering.
