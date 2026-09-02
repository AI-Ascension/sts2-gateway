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
transport failure. The minimal POC also verifies the copied protocol artifact and the fixed route
fence proof. An authorized exact-host trace now confirms external authentication, readiness, route
compatibility with the live mod, and the bounded probe effect. Concurrency isolation,
timeout/disconnect reconciliation, duplicate operation identity, and general lifecycle remain
unverified. See [TESTING.md](TESTING.md) and [COMPATIBILITY.md](COMPATIBILITY.md).

## Attached runtime slice

The first concrete gateway lane is `sts2-gateway-runtime`, a loopback-only, single-instance adapter.
It requires separate gateway and mod bearer tokens, allocates one configured identity, checks lease
and epoch headers before forwarding, and admits only the bounded `/health/ready`, state, action, and
release routes. It is consumed by the runtime MCP adapter and harness coordinator through real TCP
connections.

This lane attaches to an already running downstream listener; it does not launch, own, or recover a
game process. The fixed action remains the safe host-visible `show_runtime_probe`, not a gameplay
mutation. Source/build, controlled component-network, and exact-host forwarding evidence are
confirmed independently; process supervision, general lifecycle, and broader host/platform
compatibility remain `unverified`.
