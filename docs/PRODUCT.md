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

These are scope decisions for the public product boundary. The initialized package implements only
an in-memory control-plane core and deterministic seam fixtures; no route, field, error, timeout, or
wire shape is an accepted downstream contract until a requirement, owner, compatibility version,
and deterministic oracle are accepted.

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
transport failure. Concrete authentication, external process startup, health/readiness transport,
route compatibility, concurrency isolation, timeout/disconnect reconciliation, duplicate operation
identity, and bounded queue behavior remain unverified. Controlled host/runtime validation is a
separate authorized gate; see [TESTING.md](TESTING.md) and [COMPATIBILITY.md](COMPATIBILITY.md).
