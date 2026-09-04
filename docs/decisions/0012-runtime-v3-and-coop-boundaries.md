# ADR 0012: Runtime-v3 routing and co-op synchronization boundary

- Status: Accepted as a source-level gateway seam
- Date: 2026-09-04

## Context

Runtime-v3 adds semantic state, legal-action, transition, and recovery routes. Cooperative mode adds
peer identity and synchronization, but it must not move game semantics or model policy into the
gateway. Launch, readiness, leases, routing, fencing, restart, and shutdown remain gateway-owned.

## Decision

Expose only the fixed `/v3/instances/{id}/` route set for state, legal actions, dispatch, wait,
reobserve, and recovery. The Runtime-v3 forwarder bounds JSON bodies and responses and delegates
meaning to the host boundary. `CoopSession` is an additive, bounded peer ledger: it permits two to
four peers, one local peer, generation matching, explicit disconnect/missing-peer state, and
mutation authorization only while synchronized. It has no game-state or provider dependency.

The process supervisor accepts an injected process port and owns only handles it started. It has
bounded capacity and explicit graceful/forced stop; executable paths, credentials, profiles, and
concrete launch policy remain deployment inputs. A timeout, disconnect, stale lease, or desync is
never converted into a blind mutation retry.

## Evidence

Route allowlists, supervisor fakes, co-op tests, and static policy checks are source-derived. A
licensed process, real restart/cleanup, multi-instance isolation, host settlement, and live co-op
trace remain unverified.
