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
`RuntimeV2ForwardingPort` are explicit seams. Concrete process, scheduler, network, and persistence
access remains outside this package. The POC and Runtime-v2 checks verify checked-in copies of their
protocol artifacts as inert data; no protocol implementation path dependency is present. See
[ADR 0001](decisions/0001-gateway-ownership-and-dependencies.md),
[ADR 0002](decisions/0002-sixth-target-protocol-boundary.md), and
[ADR 0006](decisions/0006-runtime-v2-gameplay-operation-ledger.md).

## Identity, lifecycle, and fencing

The future gateway contract must distinguish caller, gateway session, game session, instance,
request, operation, lease, lease epoch, and downstream correlation namespaces. The gateway creates
or validates instance/session/lease identities; it does not borrow game-mod internal identifiers.

The proposed lifecycle vocabulary is `created`, `starting`, `ready`, `busy`, `degraded`, `stopping`,
`stopped`, `failed`, and `expired`. The gateway owns admission and successor transitions. A lease
expiry, gateway restart, instance crash, shutdown, or owner change invalidates the old epoch. The
old epoch is rejected before forwarding, and a replacement instance receives fresh identity rather
than inheriting an ambiguous record.

Accepted work survives caller timeout or disconnect as an explicit status, settled result, cancelled
result, or unknown outcome. Runtime-v2 returns `unknown` after a timeout or disconnect after write,
retains the operation ID, and permits reconciliation only through a read-only retained-receipt seam.
No blind mutation retry is allowed. Duplicate operation identities replay the retained result only
when the canonical request is identical; conflicting reuse is rejected as `idempotency_conflict`.

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
and validates a typed state request; because the attached binary has no configured host-state adapter,
it returns an explicit structured `state_unavailable` response rather than claiming its local
fallback observation is host state. Other v2 GET paths are rejected and never treated as proxy
routes. The attached binary deliberately has no authorized v2 host adapter: its v2 forwarding seam
fails closed before write, while the in-memory fake tests cover settlement, uncertainty, replay,
conflict, fencing, capacity, and artifact tamper rejection. No live gameplay mutation or host
settlement is evidenced by this route implementation.

## Runtime-v3 and co-op extension

ADR 0007 keeps Runtime-v3 routing semantic but narrow: state, legal actions, dispatch, wait,
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
non-loopback addresses and DNS names. See [ADR 0009](decisions/0009-runtime-v3-framing-and-fencing.md).

Co-op mutation snapshots require a local peer as well as at least two connected peers. The
session generation advances only when every registered peer is connected and reports the same
generation at or above the current baseline. Partial agreement, missing peers, and rollback keep
mutation suspended. This is synchronization bookkeeping, not host or multiplayer evidence.
