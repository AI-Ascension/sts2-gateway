# ADR 0003: Foundation preparation and evidence boundary

## Status

Accepted for Wave 1 preparation and Wave 2 package initialization.

## Context

The gateway tree began as a layout scaffold. Repository parity is required before product behavior,
but a manifest or empty crate would not prove lifecycle, routing, authentication, or isolation. Wave
2 now needs one real target-owned package so those control-plane invariants can be exercised without
claiming live integration.

## Decision

Wave 1 added repository governance, documentation, decisions, and repository policy tooling. Wave
2 adds exactly one non-empty `sts2-gateway` package containing an in-memory lifecycle coordinator,
identity/lease/fence types, fixed route classes, bounded opaque forwarding, and explicit injected
clock, process, readiness, transport, and lease-decision ports. Its deterministic fakes are test
fixtures only. No concrete listener, process wrapper, host object, game rule, MCP semantics,
provider/storage behavior, harness behavior, generated contract, or protocol path dependency is
added.

Static checks may establish file presence, syntax, links, license headers, workflow restrictions,
and source-size budgets. They must not be reported as proof of process startup, game readiness,
route compatibility, authentication enforcement, lease fencing, failure isolation, or host
compatibility. Later deterministic tests will use injected clocks, schedulers, processes, and
transports; controlled host tests require separate authorization and disposable state.

## Consequences

The repository has executable policy and package checks without pretending that an in-memory core is
runtime-ready. Deterministic control-plane outcomes are confirmed only within the fake seams;
external authentication, process, network, host, concurrency, and compatibility behavior remains
`unverified`. The next gateway owner must take the initialized invariants through a contract-ledger
review before adding a protocol dependency or concrete adapter.
