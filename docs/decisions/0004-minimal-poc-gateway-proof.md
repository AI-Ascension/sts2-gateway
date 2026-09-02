# ADR 0004: Minimal POC gateway proof

- Status: Accepted for the deterministic POC
- Date: 2026-09-02

## Context

The gateway is one boundary in a fake six-target vertical slice. It must prove instance identity,
lease/fence ownership, readiness, and fixed routing while remaining independent of the game-mod
implementation and the protocol implementation crate.

## Decision

Copy the `sts2-protocol/poc-v1` release-like artifact into `protocol-artifact/poc-v1` and verify
its manifest, schema identity, and golden/invalid fixtures locally. Use the existing injected
control-plane seams to allocate fake instances, reconcile one to ready, forward a fixed command
route, and reject a stale epoch or a proof from another instance before transport.

The test uses no listener, child process, network, credential, game file, or cross-repository path
dependency. The forwarded body remains opaque to the gateway; game semantics belong downstream.

## Consequences

The gateway has reviewable source/test evidence for lifecycle and fencing in the requested path.
The test does not prove live process startup, HTTP compatibility, game state, action legality, or
effect settlement. Those remain separate unverified boundaries.
