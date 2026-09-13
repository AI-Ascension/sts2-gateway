# ADR 0024: gateway-owned profile process lifecycle

- Status: Accepted for the gateway source/component boundary
- Date: 2026-09-13
- Owner: `sts2-gateway`
- Requirement: issue #50

## Context

Workflow execution needs the gateway to own the lifetime of an isolated game process. The existing
`ProcessPort` and `ProcessSupervisor` seams deliberately contain no executable paths, install
selection, user-data paths, or restart authority. A caller-provided command or an arbitrary attach
would make lease fencing and cleanup unverifiable.

## Decision

Add a closed, server-owned `ApprovedLaunchProfiles` catalog. A caller supplies only an opaque
`LaunchProfileId`; the catalog resolves it to an exact executable/install/image identity, an
isolated user-data namespace, and bounded descendant and start/stop timeout policy. The
`ApprovedLaunchProfileAdapter` passes that resolved value to an injected `ProcessPort`. No browser
or workflow field can carry a command, executable path, URL, environment, or user-data path.
Profiles require a nonzero descendant bound and positive, bounded start/stop timeouts; malformed
zero or over-limit policy values are rejected before catalog insertion.

Extend the process port with identity-bearing launch, exact identity inspection, bounded descendant
inspection, and owned-process recovery. `ProcessIdentity` records the gateway instance, process
handle, PID, birth identity, executable/install/image identity, and user-data namespace. Stop and
restart verify the complete identity and every observed descendant before acting; an identity or
scope mismatch fails closed.

`ProcessLifecycle` authenticates every request with the bound lease/fence and an authority epoch.
It persists an idempotent `LifecycleOperation` intent before invoking the process port. Duplicate
operation identities replay the retained record; a conflicting request is rejected. Launch returns
the owned identity with lifecycle state `Starting`, while a distinct `AttachExisting` action may
only use an identity retained in an earlier gateway operation. `LaunchNew` never adopts an
arbitrary PID.

The `LifecycleRecordStore` seam has deterministic in-memory and SQLite implementations. On
reconnect or service restart, an unresolved intent calls the adapter's explicit recovery seam. A
missing recovery result is `Unknown`; it is never converted into a blind relaunch. A process fault
without a transferred identity is terminal `Failed`, while stop/cleanup faults retain the exact
identity as `Blocked` for an explicit reconciliation or cleanup retry.

Restart force-stops the verified old identity, checks descendant scope, clears old ownership, and
rotates the authority epoch before launching the replacement. Requests carrying the previous
epoch are rejected before the process port is called. Failed cleanup or uncertain identity retains
the record and blocks replacement allocation.

## Compatibility and rejection/cancellation behavior

This is an additive gateway-local source/component contract at the route and trait-method
boundary. Existing `ProcessPort::start`, `inspect`, `stop`, `ProcessSupervisor`, gateway routes,
MCP framing, harness code, and protocol artifact bytes remain available. The default
identity-bearing profile method rejects legacy ports before invoking `start`; profile-aware
callers must provide an implementation that proves launch identity and cleanup. Public lifecycle
and fault enums also gained variants, so Rust consumers with exhaustive `match` expressions need
new arms (wildcard or non-exhaustive matches remain source-compatible). Consequently this is
minor/additive for wildcard-matching consumers but source-breaking for exhaustive enum consumers.
Unknown or duplicate profile IDs, malformed profile bounds, wrong image/install/user-data
identity, foreign PID/birth identity, unowned attach, stale lease/authority epoch, capacity
exhaustion, stop timeout/failure, and foreign descendants have typed fail-closed outcomes.

Caller timeout or disconnect does not cancel an accepted process mutation. The durable operation
record is the reconciliation handle. Reconciliation is read/inspect-only until the adapter proves
the prior process identity; no mutation is retried from an uncertain response. The concrete native
game executable, OS signal handling, host readiness, and production multi-instance deployment
remain outside this source/component change and are `unverified`.

## Deterministic oracle and evidence

`process_lifecycle_tests` and the focused integration fixtures use only injected clocks, stores,
lease decisions, and synthetic process ports. They prove approved-profile resolution, launch-once
duplicate replay, reconnect/recovery without relaunch, crash-before/after-creation outcomes,
wrong image/install and PID/birth rejection, stale epochs, unowned attach, stop failure and
foreign-descendant blocking, restart epoch rotation, SQLite record replay, and capacity
exhaustion. The tests do not launch a native game or claim host compatibility; all process evidence
is synthetic/source-component evidence.
