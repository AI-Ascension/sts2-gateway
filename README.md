<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-dark.svg">
  <img alt="AI-Ascension — Inspect how AI requests to a game get fenced, one Rust contract at a time. Bounded runtime host trace confirmed. Deterministic tests: confirmed." src="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-light.svg" width="100%">
</picture>

# sts2-gateway

> **AI-Ascension · tier 2: control plane · home of the public proof** — In-memory control plane for game-host instances: lifecycle, one lease per instance with epoch fencing, and fixed routes.
>
> **Status:** deterministic tests, the bounded attached-host runtime trace, the reviewed runtime-v3 Windows/Linux campaign path, and the serialized `coop-native-v1` gateway boundary are `confirmed` for their recorded evidence · native two-peer settlement, general lifecycle, and broader compatibility remain `unverified`.
> **Proof:** [45-second browser replay](https://ai-ascension.github.io/proof.html) · [Evidence ledger](https://ai-ascension.github.io/evidence.html) · [This repository on the map](https://ai-ascension.github.io/repositories.html#sts2-gateway)
> **Proof source:** [crates/gateway/tests/control_plane.rs](crates/gateway/tests/control_plane.rs) — the replay mirrors these tests.
> **Owner:** The gateway boundary owner is responsible for the lifecycle and routing control plane: instance records, leases and lease epochs, fencing, fixed forwarding policy, and cleanup.
> **Contribute:** [Organization guide](https://github.com/AI-Ascension/.github/blob/main/CONTRIBUTING.md) · [First tasks](https://ai-ascension.github.io/contributing.html)
>
> AI-Ascension is an independent project. It is not affiliated with or endorsed by Mega Crit or Valve and grants no rights to game files, assets, or marks.

Status: Wave 2 POC plus bounded runtime-adapter proof. The target-owned gateway package provides a
deterministic control-plane core and injected boundary ports; the separate runtime binary adds one
attached loopback lane. Dated evidence confirms that lane through the exact recorded host and shows
the runtime-v3 path carrying isolated Windows and Linux model-controlled campaigns to Defeat and
fresh replays. Generic process supervision, multi-instance lifecycle, native multiplayer, and
broader compatibility remain outside the demonstrated scope.

## Owner and boundary

The `sts2-gateway` boundary owner is responsible for the external lifecycle and routing control
plane: instance records, allocation, process ownership, readiness and health observation,
authentication and authorization, leases and lease epochs, fencing, fixed forwarding policy,
per-instance isolation, bounded backpressure, and cleanup.

Its intended consumers are the harness control coordinator and the thin MCP server adapter. The
gateway may address an isolated `sts2-game-mod` process at runtime, but it does not import or own
host, loader, game-rule, or game-state implementation. Operator and recovery clients are separate
control-plane consumers. Runtime communication and compile-time dependencies are distinct; see
the [architecture](docs/ARCHITECTURE.md).

The gateway does not own game rules, host objects, managed loader code, MCP semantics or tool
catalogs, model/provider execution, harness episodes or artifacts, direct game files, saves,
credentials, arbitrary proxying, or implicit remote discovery. It consumes only inert copied
`sts2-protocol/poc-v1`, Runtime-v2, semantic Runtime-v3 gameplay, `seeded-run-v1`, and accepted
`coop-native-v1` artifacts. A forwarded request must have a validated instance,
session, lease, lease epoch, route, method, and bounded body; listener reachability is not
authentication. Runtime-v2 adds only the fixed `end_turn` operation and its retained receipt ledger,
plus a typed state route that reports explicit unavailability without a host-state adapter. The
Runtime-v2 fake settlement remains distinct from the later bounded Runtime-v3 host evidence.

The POC test allocates and readies fake instances, forwards a fixed command route, and proves that
stale epochs and a proof from another instance are rejected before transport. It is a gateway
control-plane test, not evidence that a game-mod process is running or that an action settled.

## Evidence and provenance

This target is intentionally source/test bounded. The project policy and target decisions are
normative for this repository. Existing planning material is used only as a structural and
documentation exemplar. Planning and retained evidence are inputs labelled `proposed`, `inferred`, or
`unverified` unless a controlled test establishes otherwise. No reference implementation source,
proprietary game file, save, provider credential, or generated product output is copied here. The
protocol artifact is copied as explicit release-like data only.

The current state is `source-derived` from this tree and its policy files, with `confirmed`
deterministic fake-instance outcomes, a controlled component lane, an authorized exact-host runtime
trace, and the read-only `coop-synchronization-v1` executable profile. The latter reports gateway
peer metadata and never authorizes game effects. Generic process startup/supervision, isolation
under real concurrency, restart reconciliation, native multiplayer, and release behavior remain
`unverified`. See the [compatibility record](docs/COMPATIBILITY.md) and [testing plan](docs/TESTING.md).

## Local validation

Run these commands from this directory:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
(cd protocol-artifact/poc-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v2 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v3-gameplay && sha256sum -c SHA256SUMS)
(cd protocol-artifact/seeded-run-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-map-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v4-expert-rest-action && sha256sum -c SHA256SUMS)
(cd protocol-artifact/coop-native-v1 && sha256sum -c SHA256SUMS)
```

The first command is the local policy entrypoint and checks required paths, licenses, links,
workflow restrictions, Rust configuration, language restrictions, and file budgets. The package
tests exercise injected deterministic fakes and isolated synthetic loopback HTTP sockets; these
commands do not launch a game process, MCP server, provider, or real host.

## Repository map

- [AGENTS.md](AGENTS.md) is the target operating contract.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) defines ownership, trust, and dependency direction.
- [docs/PRODUCT.md](docs/PRODUCT.md) defines the gateway product boundary and non-goals.
- [docs/REPOSITORY_LAYOUT.md](docs/REPOSITORY_LAYOUT.md) records the package and boundary layout.
- [docs/TESTING.md](docs/TESTING.md) records the deterministic package suite and future security tests.
- [docs/decisions/0001-gateway-ownership-and-dependencies.md](docs/decisions/0001-gateway-ownership-and-dependencies.md)
  records gateway ownership and dependency rules.
- [docs/decisions/0002-sixth-target-protocol-boundary.md](docs/decisions/0002-sixth-target-protocol-boundary.md)
  records the current sixth-target protocol decision.
- [docs/decisions/0019-runtime-v4-expert-rest-action-route.md](docs/decisions/0019-runtime-v4-expert-rest-action-route.md)
  records the candidate Runtime-v4 expert rest-action transport boundary.
- The staged gateway investigation prompt is a discovery input, not an implementation or runtime
proof; it is maintained outside and is not copied into this repository.

## Attached runtime slice

The target now includes the standalone `sts2-gateway-runtime` binary. It is a bounded, authenticated
single-instance adapter for the first runtime slice: it exposes allocation, readiness, state, action,
and release routes on loopback, validates the configured identity/lease/epoch/correlation fence, and
forwards only the fixed runtime paths to an already attached mod listener. The MCP process reaches
this binary over its real TCP adapter; the gateway does not accept arbitrary paths or headers.

This binary does not launch or supervise a game process in this sprint. Its fixed instance and
attached downstream configuration are intentional for the vertical slice; the exact-host forwarding
and lease path is confirmed in the dated authorized trace. General process lifecycle, multi-instance
scheduling, restart reconciliation, and graceful shutdown remain `unverified`.

The Runtime-v3 profile validates the exact canonical artifact and correlated envelopes on six fixed
routes. The attached binary's fixed-instance path was exercised by the dated Windows/Linux campaign
and replay records through an already attached mod listener. It still has no lease TTL/renewal or
durable boot-epoch rotation; see the explicit restart limitation in
[COMPATIBILITY.md](docs/COMPATIBILITY.md). It cannot establish an autonomous run by itself without
the harness and provider path.

The additive `seeded-run-v1` gateway seam is present at current main
[`2b44bf347f790509c9f13378c89719d09366d45b`](https://github.com/AI-Ascension/sts2-gateway/commit/2b44bf347f790509c9f13378c89719d09366d45b).
It exposes fixed instance-scoped start and read-only reconciliation routes, validates the selected
native context and content-addressed digest, retains operation identity across correlation rebinding,
and forwards only the fixed mod paths. Its copied artifact is schema digest
`5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8`, aligned with protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. Component ledger/journal evidence does not prove native
host settlement, profile/save isolation, provider execution, gameplay, or release compatibility.
See [ADR 0018](docs/decisions/0018-seeded-run-v1-gateway-boundary.md).

The additive Runtime-v4 expert routes are also implemented and validate the checked-in expert-state
and expert-action artifacts. Native host legality, settled effects, provider execution, and
end-to-end compatibility remain unverified.

The exact Runtime-v4 source/component record is [documented in the compatibility matrix](docs/COMPATIBILITY.md#runtime-v4-expert-sourcecomponent-row).

Historical source/component update (2026-09-07): at exact gateway source head `aecc9fa44c825623b3e3bbb21d130e1fe6ac9468`, the settled Runtime-v4 expert response fence binds nested observation `state_id` and `generation` to the outer response, and the dispatch transition `before_generation` to the request generation. Independent checks passed 130 workspace tests, formatting, strict policy, Clippy, and three original regression cases. Native host legality, settled effects, provider execution, cross-consumer integration, deployment, and release remain unverified.

Current default-main source/component update (2026-09-10): gateway main
[`2b44bf347f790509c9f13378c89719d09366d45b`](https://github.com/AI-Ascension/sts2-gateway/commit/2b44bf347f790509c9f13378c89719d09366d45b)
contains the Runtime-v4 expert routes and the bounded map route. Its copied expert artifacts are
aligned with protocol main `d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404` at schema digests
`0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42` and
`393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929`. This is source/component
evidence; native host legality, settled effects, provider execution, deployment, release, and live
cross-consumer compatibility remain unverified.

For opt-in coordinator-reported synchronization, supply `STS2_COOP_ROSTER` as a JSON array,
for example `[{"peer_id":"local-1","role":"local"},{"peer_id":"ally-1","role":"ally"}]`.
The configured coordinator submits bounded peer reports under control scope; the separate
MCP `coop-synchronization-v1` profile reads agreement under read scope. All peers initially
appear missing, and connected reports expire after thirty seconds. This feature reports
coordination metadata and never authorizes gameplay. See
[the route and freshness contract](docs/COMPATIBILITY.md#opt-in-coordinator-reports).

The executable synchronization profile was separately exercised through the real gateway and MCP
processes. It covered missing and partial reports, generation agreement and disagreement,
disconnect/recovery, stale leases, and rejected unknown or regressing reports, with zero downstream
game connections. This is coordinator-report transport evidence only: it does not establish native
peer identity, shared game actions/effects, or multiplayer gameplay. See the [MCP executable
evidence](https://github.com/AI-Ascension/sts2-mcp-server/blob/main/docs/evidence/coop-synchronization-20260906.md).

The additive `runtime-map-v1` route exposes a bodyless `GET
/v1/instances/{instance_id}/map-snapshot` request and forwards only the fixed downstream map
snapshot path. It validates the corrected visible-map artifact, identity/generation fence, bounded
graph, and independent navigation bindings before returning data. The route is source/component
evidence; live map freshness and visualizer rendering remain unverified. See
[ADR 0017](docs/decisions/0017-runtime-map-read-route.md).

The candidate `runtime-v4-expert-rest-action-v1` profile adds fixed authenticated dispatch and
read-only reconciliation routes for native rest-site options and selector follow-up actions. The
gateway validates the exact candidate artifact, forwards only the fixed mod paths, and retains
selector catalogs so completed choices must have prior admission context. The two serialized
producer-shaped lifecycles and 22 mutation fixtures are deterministic source/component evidence;
the profile remains unadmitted and does not establish a native producer, host effect, MCP or
harness consumer, deployment, or release compatibility. See
[ADR 0019](docs/decisions/0019-runtime-v4-expert-rest-action-route.md).

Current default-main source/component map update (2026-09-10): gateway main
`2b44bf347f790509c9f13378c89719d09366d45b` contains the bounded map route and its copied
`runtime-map-v1` artifact. The producer pin is merged protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404` at schema digest
`ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b`. This records current
source/component identity and copied-artifact scope; live map freshness, native map visibility,
navigation, gameplay, release, and publication remain unverified.

The accepted `coop-native-v1` gateway consumer is present at the current source head. It exposes
fixed observation, legal-catalog, local-action, shared-vote, peer-rejoin, and same-operation
recovery routes under `/v1/instances/{instance_id}/coop/native/`. The copied schema digest is
`2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629`; the consumer checks closed
envelopes, duplicate members, caller and lease identity, operation lineage, catalog/effect/receipt
relations, settled generations, and recovery kind before forwarding to the six fixed mod paths.
This is source/component and synthetic transport evidence. Native peer admission, host settlement,
two-peer gameplay, checksum convergence, provider execution, deployment, and release compatibility
remain unverified. See [ADR 0021](docs/decisions/0021-coop-native-gateway-consumer.md).
