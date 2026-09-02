# ADR 0005: Attached loopback runtime slice

- Status: Accepted for the bounded component slice; process and host compatibility remain unverified
- Date: 2026-09-02

## Context

The in-memory gateway core proves lifecycle and lease policy but has no real TCP peer. A full process
supervisor would add launch, port reservation, restart, shutdown, and profile contracts before the
first cross-boundary trace exists. The next sprint needs a narrow gateway lane that preserves the
authority boundary and can be exercised against a disposable downstream.

## Decision

Add `sts2-gateway-runtime`, a standalone single-instance adapter. It binds loopback, requires a
gateway bearer token and a separate downstream mod token, and admits only allocation, readiness,
state, action, and release operations. Allocation validates the configured instance/caller/session;
state/action/release require the active lease and exact instance, caller, session, lease ID, lease
epoch, and correlation headers. Forwarding uses fixed downstream paths and bounded HTTP bodies and
responses. The configured instance never falls back to another target.

The adapter attaches to an already running mod listener. It does not launch or supervise a game
process, persist a registry, provide multi-instance scheduling, or define game semantics. The safe
`show_runtime_probe` action and its `runtime-v1` witness remain owned by the downstream host boundary.

## Consequences and evidence

The real gateway TCP path can be tested with a synthetic downstream and the result classified as
component integration evidence. Source/build and synthetic network checks are `confirmed` when their
oracles pass. Process lifecycle, managed mod interoperation, host-thread behavior, game effect,
disposable profile cleanup, and gameplay mutation remain `unverified` until separately authorized.
