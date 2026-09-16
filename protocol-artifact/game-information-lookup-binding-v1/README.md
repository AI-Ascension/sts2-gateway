# `game-information-lookup-binding-v1`

This artifact defines the authoritative initial lookup-binding discovery and observation contract for
the game-information lookup transport. It is owned by `sts2-protocol`; the named consumers are
`sts2-harness`, `sts2-gateway`, and `sts2-mcp-server`. The artifact carries no route, transport, host
access, clock, credential, registry, lifecycle, persistence, or mutation behavior.

The normative rules are `LBR-1` through `LBR-12` in
`docs/game-information-lookup-binding-contract.md`; this artifact is the machine-checkable witness of
those rules.

This copied consumer artifact is pinned to protocol source commit
`843fddbd3b5875d01d7f99d3433f872b0a4d7681`. Its conformance metadata records the reviewed
gateway (`b6b94bf`), MCP (`2567cd3`), and game-mod (`a8ccb8b`) source pins. Those pins bind the
source-level route and producer assumptions; they do not establish a registered deployment, host
extraction, or live agent outcome.

## Stable binding identifier

A binding is identified by `binding_id`: the lowercase SHA-256 hex digest of the canonical identity
input. The identity input is exactly the tuple the lookup owner already compares when deciding
whether two bindings share an owner:

| Member | Owning boundary |
| --- | --- |
| `project_id`, `run_id`, `episode_id`, `agent_id` | `sts2-harness` scope |
| `game_profile` | `sts2-game-mod` content revision |
| `content_manifest_id` | `sts2-game-mod` content revision |
| `locale` | `sts2-gateway` |
| `authority_epoch` | `sts2-harness` |

`instance_id` is deliberately **not** part of the digest. The digest is computed over compact UTF-8
JSON with object members sorted lexicographically:

```
{"agent_id":"agent-3","authority_epoch":7,"content_manifest_id":"content-1","episode_id":"episode-7","game_profile":"sts2-native-v1","locale":"en-US","project_id":"proj-1","run_id":"run-42"}
```

That input yields `58fea90991138ea6fb635df1f5eadd08973ec63eba456d135578677ffee61cfc`. A consumer
recomputes `binding_id` from the declared members; a producer-supplied `binding_id` is never trusted.

## Shapes

`lookup_binding_discovery_response` reports the pre-observation state, the required capability
(profile and schema digest), and no observation. `lookup_binding_observation_response` reports one of
`observed`, `reobserve_required`, or `reobserve_exhausted`. `error_response` reports one closed error
code. Discovery precedes observation: a consumer must be able to discover a binding before it may
observe it.

An observation is bound to the same `binding_id` it was observed under. A retained observation whose
`state_generation` falls behind the context's retained generation is stale, and the consumer discards
it and re-observes under the same `binding_id` with a new `observation_id`. Re-observation never
re-discovers: a changed identity input requires a fresh discovery. When re-observation is impossible
the binding fails closed as `reobserve_unavailable` with `observation_state` of `reobserve_exhausted`
and at least one recorded attempt; an exhausted binding must never fall back to a stale or fabricated
observation.

## Evidence

The goldens and field vectors are synthetic values. The Rust schema witness validates the same bytes
and vector IDs as the JavaScript wire witness. This is protocol/serialization evidence only. The wire
or transport route that serves discovery, consumer source adoption, gateway/MCP tool registration,
host extraction, live snapshots, and end-to-end agent reachability remain unverified until the named
owners pin this digest and the route decision is recorded.
