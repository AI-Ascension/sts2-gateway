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

The lifecycle store also persists one authoritative ownership row per instance, separate from
request history. The row carries the gateway-issued sequence, approved profile, and an optional
exact process identity; an identity-less row is an active capacity reservation while recovery is
uncertain. Rejected requests never replace that row. Fresh operations receive a server-issued
sequence, so caller-selected operation IDs remain idempotency keys rather than ordering authority.
An approved user-data namespace may be reserved by only one active instance; a missing or stale
profile record is treated as a conflict because isolation cannot be proven. The on-disk SQLite
store holds an exclusive coordinator lock for its lifetime, fencing competing lifecycle
coordinators before they can admit process effects.

Retained operation records have an explicit `max_records` budget in `ProcessLifecycleConfig`.
Records are not silently evicted: an exact duplicate may replay at capacity, while a fresh
operation is rejected before any process-port effect once the budget is full. Startup counts the
durable rows before loading them and fails closed when the persisted set exceeds the configured
budget. This retention rule preserves replay/idempotency at the cost of requiring an operator to
provision a larger budget for additional history.

The `LifecycleRecordStore` seam has deterministic in-memory and SQLite implementations. On
reconnect or service restart, an unresolved intent calls the adapter's explicit recovery seam. A
missing recovery result is `Unknown`; it is never converted into a blind relaunch. Only an
explicit pre-start rejection (`StartRejected`, `ProfileRequired`, or `ProfileNotApproved`) is
terminal `Failed`; every other launch fault is `Unknown` with its durable capacity reservation
retained and an optional recovered identity attached. Stop/cleanup faults retain the exact
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
exhaustion, stop timeout/failure, and foreign descendants have typed fail-closed outcomes. The
gateway persists a server-issued ordering sequence for recovery; caller operation IDs remain
idempotency keys and need not define chronology.

Caller timeout or disconnect does not cancel an accepted process mutation. The durable operation
record is the reconciliation handle. Reconciliation is read/inspect-only until the adapter proves
the prior process identity; no mutation is retried from an uncertain response. The concrete native
game executable, OS signal handling, host readiness, and production multi-instance deployment
remain outside this source/component change and are `unverified`.

The ownership row and `max_records` setting are additive gateway-local persistence changes.
Existing stores that do not implement the ownership seam fail closed rather than silently
reconstructing mutable authority from request history. A full budget returns
`LifecycleError::CapacityExceeded`; retained records and exact duplicate replay remain available.

## Deterministic oracle and evidence

`process_lifecycle_tests` and the focused integration fixtures use only injected clocks, stores,
lease decisions, and synthetic process ports. They prove approved-profile resolution, launch-once
duplicate replay, reconnect/recovery without relaunch, crash-before/after-creation outcomes,
wrong image/install and PID/birth rejection, stale epochs, unowned attach, stop failure and
foreign-descendant blocking, restart epoch rotation, SQLite record replay and disk reopen,
single-writer fencing, profile namespace isolation, and capacity exhaustion before a process
effect, identity-less recovery quarantine, rejected-request ownership preservation,
server-sequence ordering, transient inspection/descendant retention, and repeated read-only
recovery until an exact replacement identity is found. The tests do not launch a native game or
claim host compatibility; all process evidence is synthetic/source-component evidence.

## Amendment — durable ownership and retention correction

The issue #50 review correction keeps uncertain recovery `Unknown` and capacity-reserving, retains
authorized cleanup identity before fallible stop/restart verification, distinguishes failed
observation from confirmed parent-and-descendant exit, and retries read-only recovery for an
identity-less replacement. It adds the durable ownership row, server-issued sequence, and explicit
record budget described above. No route, game-mod contract, MCP framing, harness behavior, native
adapter, or release claim changes.
