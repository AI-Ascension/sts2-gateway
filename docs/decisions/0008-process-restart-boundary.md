# ADR 0008: bounded process restart boundary

- Status: Accepted as a source-level gateway seam
- Date: 2026-09-04

## Context

The gateway owns the lifetime of an isolated game process. Recovery needs an explicit restart
operation, but the gateway foundation must not know executable paths, profiles, host objects, or
process implementation details.

## Decision

`ProcessSupervisor::restart` operates only on a handle previously started and owned by that
supervisor. It force-stops the old handle, removes that ownership, and starts one replacement with
the same opaque instance identity. A stop or start failure is returned as a typed process fault;
after a successful stop followed by a failed replacement start, no handle remains owned and the
caller must fail closed or allocate a fresh instance. The method does not select a game action,
reuse an old process handle, or silently fall back to another instance.

Concrete executable, profile, credential, readiness, and persistence behavior remains behind the
injected `ProcessPort`. Runtime restart, lease-epoch rotation, and target-game recovery require a
separate deployment adapter and remain unverified.

## Evidence

The deterministic supervisor test proves replacement ownership and the old-handle boundary. It is
source/component evidence only; no live process was restarted.
