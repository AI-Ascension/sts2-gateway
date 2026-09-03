# Gateway product boundary

## Purpose

The gateway will provide one authenticated control/data plane for explicitly identified isolated
game-mod instances. It will give the harness and MCP adapter a stable place to allocate, attach,
observe, route, fence, recover, and release an instance without moving game authority out of the
game-mod.

## Owned scope

The accepted gateway scope is:

- instance identity, session binding, process ownership, allocation, and cleanup;
- lifecycle state, readiness and health observation, crash/recovery quarantine, and shutdown;
- lease issuance/renewal/revocation, lease epochs, stale-operation fencing, and idempotency rules;
- the bounded Runtime-v2 operation ledger, fixed `end_turn` forwarding seam, retained receipts, and
  explicit accepted/settled/rejected/unknown/cancelled outcomes;
- caller authentication and authorization for control and fixed data routes;
- fixed method/path/header/body allowlists and target revalidation;
- per-instance isolation, bounded queues/payloads, backpressure, and sanitized diagnostics; and
- independent gateway API compatibility and release metadata.

These are scope decisions for the public product boundary. The generic package implements an
in-memory control-plane core and deterministic seam fixtures. The accepted sprint slice adds a
separately documented, fixed attached runtime adapter with its own bounded route/identity oracle;
it is not a general lifecycle or public-release contract.

## Consumers and non-goals

The harness consumes gateway control operations as coordinator, and the MCP server consumes the
authenticated fixed data contract as an adapter. The game-mod is a downstream runtime peer. The
gateway does not own game rules, game state, host objects, loader/ABI code, MCP framing or tool
catalogs, model/provider calls, episodes, trajectories, replay, scoring, saves, profiles, or
arbitrary proxying. It does not contact a provider or discover remote game processes implicitly.

## Evidence and next gate

The repository has an initialized control-plane package. Its local evidence includes static policy,
format, lint, build, and deterministic fake-instance tests for allocation, readiness, process
inspection/crash, expiry, wrong instance, stale epoch, cleanup, shutdown, bounded forwarding, and
transport failure. The POC and Runtime-v2 tests verify copied protocol artifacts; the v2 fake lane
also covers exactly-once application, retained-receipt reconciliation, duplicate replay, conflict,
stale fencing, cancellation, persistence checkpoint failure, bounded capacity, and restart recovery;
the attached journal also fails closed on same-path process contention.
An authorized exact-host trace confirms only the older runtime-v1 probe. Runtime-v2 live gameplay
settlement, downstream crash recovery, lease-epoch rotation, production persistence, and supervisor-
level concurrency isolation remain unverified. See [TESTING.md](TESTING.md) and
[COMPATIBILITY.md](COMPATIBILITY.md).

## Attached runtime slice

The first concrete gateway lane is `sts2-gateway-runtime`, a loopback-only, single-instance adapter.
It requires separate gateway and mod bearer tokens, allocates one configured identity, checks lease
and epoch headers before forwarding, and admits only the bounded `/health/ready`, v1 state/action,
release, and fixed Runtime-v2 state/action/operation routes. Its v2 state route returns a typed
request plus explicit `state_unavailable` when no host-state adapter is configured; it does not
expose arbitrary GET proxying. It is consumed by the runtime MCP adapter and harness coordinator
through real TCP connections.

The attached adapter uses a single bounded FIFO worker queue configured by
`STS2_RUNTIME_V2_QUEUE_CAPACITY` (1 through 64). Authenticated overflow returns a typed HTTP 429
without downstream dispatch. The fixed metrics route reports sanitized queue and lifecycle
counters, and the lease-fenced shutdown route closes admission and explicitly cancels queued work.
These are component lifecycle controls, not a multi-instance supervisor or production signal/restart
implementation.

Gateway authentication additionally supports explicit `read`, `mutate`, and `control` scopes,
current-token expiry, and one expiring previous-token overlap for rotation. Missing, invalid, or
expired credentials fail with 401 and insufficient scopes fail with 403 before queue admission. The
attached process still relies on an external credential issuer/secret owner and does not claim
production revocation or downstream-token rotation.

This lane attaches to an already running downstream listener; it does not launch, own, or recover a
game process. The runtime-v1 fixed action remains the safe host-visible `show_runtime_probe`. Runtime-v2
uses `end_turn` through its fixed authenticated forwarding seam and optional bounded journal, but the
attached binary has no verified host-capable mod adapter and therefore makes no live gameplay mutation
claim. Source/build, controlled component-network, and exact-host v1 forwarding evidence are confirmed
independently; process supervision, general lifecycle, v2 settlement, and broader host/platform
compatibility remain `unverified`.
