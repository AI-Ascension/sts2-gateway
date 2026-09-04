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

The ledger's monotonic-observation repair is a patch to the existing freshness invariant; it does
not change Runtime-v2 artifact bytes. Historical receipts keep their own generation and are distinct
from the newest admission observation. Corrupt/inconsistent checkpoint generations fail closed.

The generic control-plane recovery correction is a patch to failed-start capacity and expiry error
reporting, not a new route or wire field. Failed starts now leave no queryable allocation because no
allocation identity was returned; consumed IDs are not reused. Expiry reconciliation returns the
existing `ProcessStop` error on failed cleanup instead of incorrectly reporting `Expired`. Callers
must retain the already-issued allocation identity and invoke authorized `cleanup` after the fault is
resolved. `ProcessPort::start` transfers a handle only on success; partial-start cleanup on error is
the port's responsibility. No concrete process adapter is validated by this correction.

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
| Runtime-v2 ledger and attached adapter | Owner-local ledger plus fixed synthetic TCP downstream | Rust gates, byte-level artifact verification, deterministic fault tests, and isolated component restart trace | Fixed state/action/operation forwarding, bounded optional journal recovery with exclusive path ownership, exact bearer check, and synthetic route behavior confirmed; live downstream action settlement, lease-epoch rotation, multi-instance isolation, and host compatibility unverified |

The adapters' fixed configurations are sprint boundaries, not general lifecycle support claims. The
attached Runtime-v2 process accepts an optional bounded version-1 journal and a retained-operation
capacity of 1 through 64. The service owns an exclusive stable lock sibling for the configured
journal path and fails closed when another process already holds it; each instance must use a distinct
path. A journal identity or lease-epoch mismatch fails closed; an in-flight or
accepted operation restored after restart becomes explicit `unknown` and is reconciled read-only.
Clients must not blindly resend an unknown action. A future compatibility promotion must add exact
process ownership, readiness, lease-epoch rotation, multi-instance, downstream crash, and
disposable-host evidence. The attached process also accepts a bounded FIFO queue-capacity setting
from 1 through 64, exposes sanitized metrics, and supports a lease-fenced shutdown route. These
additions are component lifecycle controls; they do not establish process ownership, signal
handling, global scheduling, or host compatibility. `STS2_MCP_SESSION_ID` defaults to the gateway
session and may be set independently; every lease-protected request must then carry the matching
`x-mcp-session-id` value.
