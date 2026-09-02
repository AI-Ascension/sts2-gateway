# Compatibility policy

## Current evidence

The `protocol-artifact/poc-v1/` directory is an offline release-like copy consumed by the POC
test. Exact artifact identity and fixture bytes are confirmed locally; this does not establish
compatibility with a live game-mod, host, network, or runtime.

The copy is synchronized verbatim from the normative protocol artifact at source revision
`cad3c85d3cba3363ad387f9c26a3c3cac2782267` (protocol PR #2). Its manifest binds the package
schema path to the canonical source digest, and the gateway's copied source/package paths are
verified separately.

The Runtime-v2 copy is synchronized from the protocol handoff at commit
`8d4b2f574cf860a71f2a5e4ce3308ac069cb1527`. Its source and package schema bytes both have digest
`f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2`, and the local
`protocol-artifact/runtime-v2/SHA256SUMS` inventory passes. This proves artifact-copy integrity,
not consumer or host compatibility.

This target has repository governance, one target-owned control-plane package, a separate attached
runtime binary, and deterministic fake tests. The controlled component lane uses synthetic data,
while the authorized exact-host lane uses the packaged game-mod listener. Attached forwarding,
authentication, lease fencing, and the bounded probe path are confirmed for that exact host; external
process supervision, real concurrency isolation, host gameplay, and general compatibility remain
`unverified`.

Static policy results may establish configuration and source compatibility with the pinned Rust
toolchain. They do not establish compatibility with a game or a historical implementation.

## Independent version axes

Keep these values separate and record each in a future compatibility matrix:

| Axis | Owner | Compatibility question |
| --- | --- | --- |
| Rust/toolchain | repository | Can the governance and gateway code build with the declared MSRV? |
| Gateway API | gateway | Do control/data requests, errors, identity, and lease rules match? |
| Game-mod HTTP contract | game-mod | Does the fixed downstream route contract match? |
| Game host/loader | game-mod | Can the host boundary load and execute in the claimed environment? |
| MCP revision | MCP server | Can the adapter map its accepted calls to the gateway? |
| Harness client | harness | Can the coordinator preserve instance and lease lineage? |
| Shared protocol | protocol owner | Do neutral contract versions and fixtures remain compatible? |

One axis must not be inferred from another. A host update does not automatically change the gateway
API, and a successful gateway acknowledgment does not prove game state or effect settlement.

## Change classification

- **Patch:** correction that preserves accepted identity, route, lease, error, and timing behavior.
- **Minor:** additive bounded field or operation with an older-client behavior defined.
- **Major:** changed lifecycle state, route/method, auth scope, lease/fence rule, error semantics,
  timing guarantee, or removal requiring migration.

Every public change needs a requirement, deterministic conformance case, migration note where
needed, and an updated compatibility record. Boundary-specific behavior remains local unless a
versioned, neutral protocol contract is accepted by its named consumers.

## Evidence record

Future records must include exact target revision, toolchain, OS/architecture, game/mod/host versions
when applicable, contract digests, instance and lease identities, clock/seed, disposable fixture
status, sanitized commands/logs, and evidence level. Use `confirmed` only for an authorized controlled
test; use `source-derived`, `inferred`, `proposed`, or `unverified` precisely.

## Runtime adapter row

| Adapter | Downstream | Current evidence | Result |
| --- | --- | --- | --- |
| `sts2-gateway-runtime` | Attached loopback runtime-v1 listener | Rust gates, synthetic TCP lane, and authorized exact-host trace | Attached forwarding and lease path confirmed for STS2 v0.107.1 Windows x86-64; general lifecycle and gameplay unverified |
| Runtime-v2 ledger | Owner-local deterministic forwarding fake | Rust gates, artifact checksum, and deterministic fake tests | Source/ledger behavior confirmed; live downstream action settlement, restart retention, and host compatibility unverified |

The adapters' fixed configurations are sprint boundaries, not general lifecycle support claims. The
Runtime-v2 ledger retains entries only in memory until capacity is reached; it does not evict entries
or persist them. A restart loses retained receipts and must establish a new lease epoch before any
new work. Clients must not retry an unknown action after restart; they need an externally retained
receipt or a new operation identity under a newly established context. A future compatibility
promotion must add exact process ownership, readiness, shutdown, restart, multi-instance, and
disposable-host evidence.
