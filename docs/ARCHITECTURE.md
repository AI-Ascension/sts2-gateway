# Gateway architecture

## Decision first

`sts2-gateway` is one external control/data-plane boundary for explicitly identified game
instances. It owns lifecycle, process ownership, identity, authentication, leases, fencing,
readiness, health, fixed routing, isolation, bounded backpressure, and cleanup. It is not a game
authority, host adapter, MCP server, model runner, or generic reverse proxy.

This document describes the accepted boundary and the initialized package seams. The package is a
small in-memory control-plane core; its attached `runtime-v1` adapter has a confirmed exact-host
trace, while the separate `runtime-v2` gameplay-operation path is deterministic fake/source evidence
only and live host settlement remains `unverified`.

## Runtime topology

```text
operator or harness coordinator ── gateway control plane
                                      │ instance records, leases, health
MCP client ── sts2-mcp-server ────────┘
                    │ authenticated fixed data request
                    v
             sts2-gateway data plane ── isolated sts2-game-mod ── game host
```

The MCP server remains a thin protocol adapter. The harness owns coordination, model/provider
execution, runs, episodes, trajectories, replay, scoring, and artifacts. The game-mod owns host
translation and authoritative game effects at the host boundary. The game-core remains
host-independent and free of transport, process, filesystem, and concrete-host dependencies. A
gateway failure or game crash is contained to the
affected instance; it never authorizes silent fallback to another instance.

## Control and data planes

The control plane allocates and attaches instances, records process ownership, observes health and
readiness, renews or revokes leases, performs recovery, and closes admission during shutdown.
The data plane forwards only an approved operation to the target selected by the control plane.
Each request must bind caller identity, session identity, instance identity, request/correlation
identity, lease identity, lease epoch, fixed method/path, allowed headers, and bounded body. Runtime-v2
adds a bounded operation ledger whose key is instance + session + lease + epoch + operation, and
retains the canonical request identity with its result.

The gateway terminates caller authorization and emits only declared downstream identity data. It
does not pass through arbitrary credentials or headers, infer a target from a port, expose an
arbitrary URL, or interpret game-rule payloads.

The attached Runtime-v2 adapter applies a gateway-local opaque credential policy before queue
admission. Current and optional previous bearer values have bounded scopes and optional expiry, and
the previous value supports a controlled rotation overlap. Gateway credentials never cross the
forwarding seam; the separately configured mod credential is emitted only on fixed downstream paths.

## Ownership and dependency graph

Runtime communication and compile-time dependencies are separate:

```text
Runtime:       harness -> MCP server -> gateway -> isolated game-mod -> host

Compile time:  gateway -> owner-local gateway contracts
               gateway -/-> game-mod implementation, host, MCP implementation, harness internals
               gateway -> sts2-protocol only for an accepted neutral contract
```

The initialized package keeps lifecycle records and lease/fence policy local and testable without
I/O. Its `Clock`, `ProcessPort`, `ReadinessPort`, `TransportPort`, `LeaseDecisionPort`, and
`RuntimeV2ForwardingPort` are explicit seams. The attached runtime owns its bounded optional journal
adapter and its process-lifetime exclusive journal lock at the process boundary; the generic package
does not acquire filesystem persistence. The
POC and Runtime-v2 checks verify checked-in copies of their protocol artifacts as inert data; no
protocol implementation path dependency is present. See
[ADR 0001](decisions/0001-gateway-ownership-and-dependencies.md),
[ADR 0002](decisions/0002-sixth-target-protocol-boundary.md),
[ADR 0006](decisions/0006-runtime-v2-gameplay-operation-ledger.md),
[ADR 0007](decisions/0007-runtime-v2-journal-and-boundary-hardening.md), and
[ADR 0010](decisions/0010-runtime-v2-mcp-session-fence.md).

## Identity, lifecycle, and fencing

The future gateway contract must distinguish caller, gateway session, MCP session, game session,
instance, request, operation, lease, lease epoch, and downstream correlation namespaces. The gateway
creates or validates instance/session/lease identities; it does not borrow game-mod internal
identifiers. The attached runtime additionally fences the configured MCP session in
`x-mcp-session-id` without placing that transport identity in the frozen Runtime-v2 envelope or
forwarding it to the game-mod.

The proposed lifecycle vocabulary is `created`, `starting`, `ready`, `busy`, `degraded`, `stopping`,
`stopped`, `failed`, and `expired`. The gateway owns admission and successor transitions. A lease
expiry, gateway restart, instance crash, shutdown, or owner change invalidates the old epoch. The
old epoch is rejected before forwarding, and a replacement instance receives fresh identity rather
than inheriting an ambiguous record.

Accepted work survives caller timeout or disconnect as an explicit status, settled result, cancelled
result, or unknown outcome. Runtime-v2 returns `unknown` after a timeout or disconnect after write,
retains the operation ID, and permits reconciliation only through a read-only retained-receipt seam.
No blind mutation retry is allowed. Duplicate operation identities replay the retained result when
the canonical operation request is identical; its per-attempt correlation is rebound to the retry.
Conflicting reuse is rejected as `idempotency_conflict`.

In the generic control-plane core, a failed `ProcessPort::start` transfers no process handle. The
process port must clean partial-launch resources before returning an error. The gateway removes the
unreturned allocation record and restores capacity without reusing its consumed instance or lease
identity. It does not guess a handle to kill. Once a start has succeeded, cleanup responsibility stays
with the gateway: failed forced expiry cleanup revokes the lease, retains the process handle in
`failed`, and makes `reconcile` return `ProcessStop` for an explicit authorized cleanup retry.

## Trust and failure boundaries

Caller input is untrusted. Gateway authorization, fixed routing, lease/fence checks, queue limits,
and redacted diagnostics are enforcement responsibilities. The game-mod remains authoritative for
game state and effect settlement. The gateway reports downstream acceptance separately from
readiness and settlement. Malformed state, missing process ownership, mismatched identity, expired
lease, queue overload, timeout, disconnect, crash, and shutdown fail closed with stable sanitized
errors.

## Initialized seams and future adapters

The initialized package currently provides:

- a monotonic `Clock` seam;
- `ProcessPort` for launch, observation, graceful stop, and forced cleanup;
- `ReadinessPort` for readiness/health observation;
- `TransportPort` for fixed route classes and bounded opaque payload forwarding; and
- `LeaseDecisionPort` for identity, expiry, and epoch-fence decisions; and
- `RuntimeV2ForwardingPort` for the fixed `end_turn` dispatch and read-only receipt lookup.

Lifecycle records, allocation, admission, shutdown, cleanup, and state transitions are owned by
the gateway core itself. Future adapters may add scheduling, safe port reservation, persistence,
authentication credential verification, and restart reconciliation only behind reviewed seams.

Each seam needs a named consumer, resource limit, shutdown path, deterministic fake, and evidence
level. The package has deterministic fakes for its current tests; they are not runtime adapters.
The attached Runtime-v2 adapter adds fixed TCP forwarding, a bounded optional journal with an
exclusive process-lifetime lock, and a single-instance retained-operation capacity. It now also has
one bounded FIFO admission queue,
authenticated metrics, and an owner-controlled shutdown route; those additions do not provide a
process supervisor or four-instance scheduler.

## Attached runtime adapter

ADR 0005 adds a separate binary under `crates/gateway/src/bin/`. `sts2-gateway-runtime` is an
attached single-instance adapter, not a replacement for the generic `Gateway` control-plane core.
It binds a configured loopback address, terminates the gateway bearer token, handles one configured
allocation/lease, and validates the complete instance/caller/session/lease/epoch/correlation fence
before data forwarding. It routes only `/api/v1/runtime/state` and `/api/v1/runtime/action` to the
fixed downstream paths, plus readiness and release operations.

The adapter uses bounded HTTP parsing and a separate mod bearer token. It does not launch a process,
reserve a port, persist a registry, choose among instances, or perform graceful signal shutdown;
those lifecycle behaviors remain owned by the generic gateway boundary and are unverified for this
lane. The authorized exact-host trace confirms downstream readiness, forwarding, lease fencing, and
the `show_runtime_probe` witness. The action is a host-visible integration probe, not game-rule
authority.

The same adapter owns the fixed Runtime-v2 routes `GET /v2/instances/{instance_id}/state`, `POST
/v2/instances/{instance_id}/action`, and `GET
/v2/instances/{instance_id}/operations/{operation_id}`. The operation routes require the full copied
Runtime-v2 envelope, exact lease/correlation headers, and bounded ledger. The state route constructs
and validates a typed state request, forwards it to the configured authenticated mod endpoint, and
returns `state_unavailable` when no valid state can be obtained; its local fallback observation is
never presented as host state. Other v2 GET paths are rejected and never treated as proxy routes.
The binary includes fixed HTTP state/action/receipt forwarding, with tests for settlement,
uncertainty, replay, conflict, fencing, capacity, and artifact tamper rejection. This source and
synthetic component wiring is not evidence of live gameplay mutation or host settlement.
The attached adapter additionally exposes the
authenticated `GET /v2/instances/{instance_id}/metrics` route and the lease-fenced `POST
/v2/instances/{instance_id}/shutdown` route. Its listener authenticates before enqueueing into a
bounded FIFO worker queue; queue overflow and shutdown cancellation are explicit and do not retry
mutation-bearing work.

## Runtime-v3 and co-op extension

ADR 0012 keeps Runtime-v3 routing semantic but narrow: state, legal actions, dispatch, wait,
reobserve, and recovery are the only fixed route classes. The forwarder bounds JSON and delegates
profile meaning to the game-mod host boundary. The gateway never creates a legal action or infers an
effect from acceptance.

`CoopSession` is an additive peer ledger with two-to-four bounded peers, one local role, generation
matching, and explicit disconnected/missing-peer and disagreement state. Mutation authorization is
available only while synchronized. The process supervisor similarly owns only injected process
handles; its bounded restart seam stops the old owned handle before starting one replacement and
fails closed if replacement start fails. Concrete executable, profile, credential, and cleanup
adapters remain deployment inputs and require runtime evidence.

The semantic gameplay adapter now enforces the canonical copied schema, its exact digest,
authenticated header/body identity agreement, expected route kinds, operation/correlation binding,
and neutral field relationships. This does not authorize a game effect or fabricate a receipt.
The fixed five-second HTTP deadlines include partial traffic, and address configuration rejects
non-loopback addresses and DNS names. See [ADR 0014](decisions/0014-runtime-v3-framing-and-fencing.md).
Schema validation uses the pinned `jsonschema` crate (`=0.52.1`, default features off), the
gateway's only product dependency beyond `serde`; `unsafe_code = "forbid"` still applies to every
workspace crate. See [ADR 0015](decisions/0015-jsonschema-dependency-acceptance.md).

Co-op mutation snapshots require a local peer as well as at least two connected peers. The
session generation advances only when every registered peer is connected and reports the same
generation at or above the current baseline. Partial agreement, missing peers, and rollback keep
mutation suspended. This is synchronization bookkeeping, not host or multiplayer evidence.

The attached adapter enforces absolute HTTP deadlines, strict framing, literal-loopback address
configuration and terminal in-process release admission. Runtime-v2 exact receipt replay is
separate from new-action generation admission; read-only reconciliation polls accepted or unknown
work without redispatch, and historical receipts cannot rewind current observation. See
[ADR 0011](decisions/0011-attached-runtime-hardening.md). The queued executable includes Runtime-v2 journal recovery, but still has no timed lease renewal,
durable boot epoch, or concrete process supervisor; the co-op and supervisor library seams are
local prototypes and are not connected to runtime admission or co-op wire serialization.

Both gateway and mod endpoint settings require numeric loopback `IP:port` socket addresses;
wildcard/non-loopback addresses and DNS hostnames fail configuration. This plaintext attached lane
does not expose a remote mode. HTTP frames and downstream exchanges use absolute deadlines.

Runtime-v2 journal recovery requires continuity of the configured identity and downstream receipts.
Restart fencing remains an integration gate; do not reuse stale ownership after a gateway or host
restart. A new ownership context requires a fresh configured session, lease, and epoch.
Within one service lifetime, release and shutdown permanently revoke its configured lease. Further
allocation fails closed rather than reactivating the old epoch; new ownership requires a separately
configured fresh context. Persisted cross-restart revocation remains an external coordinator gate.

## Runtime-v2 wire closure

The gateway decoder requires every frozen envelope member, including nullable members, to be
present. Explicit null is accepted where the message kind permits it; omission or unknown members
fail before ledger admission and forwarding. The gateway owns this decoding boundary; host
mutation authority and protocol artifact bytes are unchanged.

## Exo component integration

The six Exo routes share the bounded worker queue and full configured MCP-session lease fence.
State, legal-actions, wait and reobserve require read scope; dispatch requires mutate scope;
recover requires control scope. Authentication and scope rejection precede admission. The
Runtime-v2 journal retains only Runtime-v2 operations; it does not persist Exo gameplay requests.

## Coordinator-report synchronization

The attached runtime's `coop_reports` module owns a bounded roster of string peer IDs and
recent reports. `service_coop` exposes the fixed read/report routes under existing admission,
scope, lease, and queue controls. It serializes the complete `coop-synchronization-v1`
artifact for MCP without linking protocol Rust implementation. It neither calls the game nor
consults reported agreement for gameplay forwarding authority. Source labels remain explicit.
The older numeric-ID `CoopSession` prototype is separate and is not wire-consumer evidence.
ADR 0015 records trust, freshness, identity lifetime and deterministic verification.
