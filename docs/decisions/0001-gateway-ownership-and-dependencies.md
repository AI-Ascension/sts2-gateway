# ADR 0001: Gateway ownership and dependency direction

## Status

Accepted for the repository foundation and Wave 2 package initialization. Concrete runtime
adapters and external compatibility remain unimplemented and runtime-unverified.

## Context

The project separates host-independent game semantics, the game-facing mod, the external MCP
adapter, the harness, and the gateway. A gateway is useful only as an independently owned control
plane for instance lifecycle, leases, fencing, identity, routing, authentication, health, and
isolation. Turning an embedded game HTTP bridge into a generic proxy would create a confused-deputy
and make instance ownership ambiguous.

## Decision

`sts2-gateway` is the sole owner of gateway instance records and control/data-plane policy. Its
boundary includes:

- instance allocation, process ownership, start/stop/recovery, readiness, and health;
- caller, session, instance, request, lease, and lease-epoch identity mapping;
- lease expiry, generation fencing, admission, shutdown, and cleanup;
- authenticated fixed route/method/header/body allowlists;
- per-instance isolation, quotas, bounded queues, and failure containment; and
- sanitized diagnostics and operator-only control operations.

The gateway must not own game rules, host objects, loader/ABI translation, MCP semantics, model
execution, harness artifacts, saves, or arbitrary proxying. Runtime communication is
`harness -> MCP server -> gateway -> isolated game-mod -> host`; operator and harness lifecycle
calls enter the gateway control plane. Compile-time dependencies point to owner-local gateway
types, with a future dependency on `sts2-protocol` permitted only for an accepted neutral contract.
The gateway does not depend on game-mod implementation crates, concrete host types, or MCP server
implementation.

Clock, scheduler, process, transport, and persistence behavior is injected behind explicit ports.
No request may be forwarded without a validated target and fresh lease fence. A timeout or
disconnect yields an explicit reconciliation state; a crash never silently reroutes work.

## Alternatives considered

1. Keep lifecycle in the harness: rejected because independent process ownership, lease fencing, and
   cross-client isolation need one authoritative control plane.
2. Let the gateway import the game-mod or host: rejected because it collapses runtime and compile
   boundaries and makes host compatibility a gateway concern.
3. Implement an unrestricted reverse proxy: rejected because arbitrary routes, headers, and targets
   bypass authorization and isolation.

## Consequences

The gateway can coordinate multiple explicitly identified instances without becoming a game
authority. The initialized core carries identity and fence data through its fixed transport seam
and maintains a separate control-plane policy. Concrete adapters still require deterministic fake
tests before any host-dependent validation. New shared contracts require the protocol decision in
ADR 0002 and a named owner, consumer, version, and conformance oracle.
