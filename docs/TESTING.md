# Gateway testing and evidence

## Forwarded save-profile operation persistence

Run `cargo test --locked --offline -p sts2-gateway --test save_profile_operation_recovery`
alongside the normal policy/format/Clippy/full workspace gates. The suite uses temporary
SQLite journals and a recording mod port, with no game or provider. It covers committed intent,
lost create/select acknowledgment, reopen-and-lookup recovery, original identity/baseline,
selection ordering, stale authority, corrupt storage, capacity and stale coordinators.
See [ADR 0027](decisions/0027-save-profile-operation-persistence.md). This does not enable
attached mutations or qualify physical allocation, actual launch binding or native profiles.

## Current status

The target contains the gateway control-plane package and repository tooling. Policy, formatting,
lint, build, and package tests run without a product workspace. The package tests use deterministic
fake clock, process, readiness, transport, and lease-decision seams. Runtime adapter tests also use
ephemeral loopback TCP listeners and synthetic peer threads. No game process, game host, provider,
save, or deployment is used by these checks.

## Baseline commands

Run from the target root:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
(cd protocol-artifact/poc-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v2 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-v3-gameplay && sha256sum -c SHA256SUMS)
(cd protocol-artifact/seeded-run-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/runtime-map-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/coop-receipt-query-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/coop-native-v1 && sha256sum -c SHA256SUMS)
(cd protocol-artifact/game-information-query-v1 && sha256sum -c SHA256SUMS)
```

The policy command comes first in the normal local sequence. CI runs the same commands with bounded
job timeouts. A missing toolchain or unavailable dependency is reported as unverified; it is not
converted into a passing skip. The package suite confirms only in-memory control-plane outcomes,
not runtime compatibility. The checksum command verifies the verbatim protocol artifact and its
checksum-covered conformance companion; the Runtime-v2 verifier independently calculates SHA-256
for every copied file named by `SHA256SUMS`.

The current deterministic suite covers allocation and readiness, process inspection and crash
failure, lease expiry and forced cleanup, stale epoch and wrong-instance rejection before transport,
graceful release, shutdown admission closure, bounded fixed-route forwarding, and transport/stop/
start failure reporting. It also exercises four independently identified allocated instances,
capacity exhaustion, survivor readiness after one instance is released, and wrong-instance fencing
without transport dispatch. The POC case additionally verifies the copied artifact identity while
combining readiness, fixed command forwarding, stale-epoch rejection, and wrong-instance fencing.
The Runtime-v2 case verifies the copied artifact and a bounded fake ledger for exactly-once
application, unknown-to-settled retained-receipt reconciliation, duplicate replay, canonical
conflict rejection, stale identity/epoch/generation replay and receipt fencing, cancellation, store
capacity, no-blind-retry, persistence checkpoint failure, restart recovery, and rejection of tampered
copied schema/manifest/golden bytes. The runtime binary tests the fixed typed state route's explicit
unavailable response, exact bearer authentication, bounded operation and queue-capacity
configuration, FIFO admission overload with retry guidance, authenticated metrics including
unknown-result and service-time counters, shutdown admission closure, and
arbitrary-v2-GET denial. The journal adapter also tests exclusive process-lifetime ownership of a
configured journal path and can sync its parent directory after atomic replacement where supported.
The T12 authority tests additionally prove owner boot/fence admission, independent recovery-domain
capabilities, fail-closed implicit workflow calls, retained-receipt gating, read-only reconciliation,
duplicate replay without a second dispatch, and workflow restore rejection for missing or changed
boot identity. These are gateway source/component checks; they do not prove durable identity
issuance, host restart continuity, native host compatibility, or downstream settlement.
The auth component additionally covers expired credentials, route scopes, and previous-token
rotation overlap; these tests use an injected test time and do not prove an external issuer or live
secret-management system. The attached runtime also tests that a mismatched configured MCP-session
header fails at the lease fence before downstream forwarding.
The control-plane fakes do not represent real processes or game hosts. Ephemeral TCP tests exercise
actual socket framing, timeouts, and forwarding only against synthetic peers.

Issue #50 adds focused process-lifecycle fixtures. The profile adapter tests reject missing and
unapproved IDs and cleans an identity mismatch; lifecycle tests prove duplicate launch replay,
authenticated stale-epoch and unowned-attach rejection, exact PID/birth/image/instance checks,
reconnect and crash reconciliation, durable intent recovery without a second launch, bounded
capacity, stop failure/timeout and foreign-descendant blocking, restart epoch rotation, closed
request serialization, and SQLite record replay. Additional review regressions cover recovery
errors and identity-less replacement reservations, rejected-request ownership preservation despite
caller-selected IDs, transient inspection and surviving descendants, identity-bearing blocked
cleanup retry, ambiguous launch failures that remain `Unknown` and capacity-reserving,
profile user-data namespace reuse rejection, identity-bearing partial-launch cleanup retention,
exclusive and transactionally fenced disk-journal coordinator ownership, independent SQLite
reopen/recovery, repeated read-only recovery, and record-budget exhaustion before a process
effect.
A launch response is `Starting` until a separate readiness adapter reports ready. These are
confirmed gateway source/component outcomes from synthetic ports and stores; they do not launch an
OS process, exercise game readiness, or establish harness/provider/native compatibility.

The attached process-lifecycle route surface is covered by `service_process_lifecycle_tests`, which
drives the real HTTP entry point (`handle_request`) rather than calling route handlers directly, so
authorization, lease fencing, routing, and dispatch are exercised as they ship. The fixture composes
through the production path (`ProcessLifecycleRuntime::compose`), so the catalog build, the durable
SQLite store open, the capacity-budget validation, and the coordinator construction are real; only
the process port is a deterministic synthetic adapter. The tests prove unconfigured refusal before
any port call, distinct `unconfigured`/`adapter_absent` reporting, capability listing only
configured profile ids, duplicate-operation replay that does not launch twice, stale-epoch,
unknown-profile, unowned-attach, and malformed/conflicting submission rejection each before any
process effect, retained-versus-absent operation lookup, the configured lease fence, the authorized
scope on both read and mutate routes, and deterministic domain-separated identity bridging. These
tests found and fixed four real defects: an inverted action field guard, unvalidated profile
entries, an instantly expired bound lease, and a digest that overflowed the store's `i64` key.
They are source-component evidence and do not launch a native game process.

The process-supervisor fixture proves that restart replaces ownership only after the old handle
is force-stopped, while its identity-bearing resolved methods reject drift and foreign descendants.
Live restart/recovery remains unverified. HTTP tests additionally cover absolute deadlines, stalled
writes, header bounds, and ambiguous framing.

Late retained settlement is checked after a newer authoritative observation: the historical receipt
remains replayable, but cannot rewind admission generation. Both accepted and unknown operations
are covered. State refresh rejects a regressed generation, and restore rejects a settled receipt
without a successor or a persisted binding older than a retained result.
The attached Runtime-v2 action profile also rejects operation IDs that cannot be reconciled by the fixed
single-segment receipt route; an ephemeral listener verifies these invalid IDs cause zero forwards.
Release/shutdown followed by allocation is rejected and leaves old lease-protected requests fenced.

The game-information consumer suite exercises the real service dispatcher with synthetic loopback
producers. It verifies exact fixed paths and caller/session/instance/lease/epoch headers for
capabilities, static list, and live detail; rejects unknown operations before a connection;
preserves a typed producer error; enforces static content and live run/snapshot scope; bounds
request/response/page/item/text/cursor data; binds cursor continuations to the complete normalized
query; and exercises timeout, cancellation, and cross-locale continuation rejection. No response
cache is implemented, and no native producer, snapshot freshness, MCP tool, harness, or host
compatibility is established.

Control-plane regression oracles include six consecutive failed starts followed by four successful
allocations at full configured capacity, without reusing failed instance/lease identities or stopping
a nonexistent handle. Expiry through `reconcile` must report forced-stop failure, retain the owned
handle, revoke forwarding, and permit explicit cleanup retry; successful expiry must report a matching
`Expired` snapshot with no retained handle. These exercise injected fake processes only.

## Future deterministic suites

For behavior beyond the initialized core, require an accepted requirement and contract ledger before
implementation. Use fake processes, injected monotonic/wall clocks, deterministic schedulers, fake
transports, bounded storage, and isolated temporary ports. Extend coverage with:

- allocation, attach, capacity limits, safe reservation, and release races;
- `created` through `ready`, `busy`, `degraded`, `stopping`, `stopped`, `failed`, and `expired`;
- process exit, readiness failure, health degradation, crash quarantine, recovery, and shutdown;
- lease issuance, renewal, expiry, epoch changes, wrong instance, stale epoch, and owner mismatch;
- authentication, authorization, fixed route/method/header/body allowlists, and arbitrary-route denial;
- caller timeout, disconnect, cancellation before/after admission, duplicate operation identity,
  bounded queue overload, and no silent drop;
- four explicitly identified instances with no response, lease, profile, or correlation bleed; and
- cleanup, join/closure, sanitized diagnostics, and restart reconciliation.

The gateway must report accepted downstream work separately from completed game effects. A timeout
or disconnect requires a status/reconciliation oracle; it must not trigger a blind mutation retry.
The attached Runtime-v2 binary can use its bounded optional journal for component restart tests.
Those tests do not establish production storage durability, mod/game crash recovery, lease-epoch
rotation, accepted-work recovery across an independently restarted downstream, or multi-instance
supervision. Its FIFO queue and shutdown route are process-component evidence only until exercised
with the authorized host supervisor.

## Evidence levels

The allocation-failure suites run in `sts2-gateway-runtime`. They exercise an
elapsed monotonic deadline, substituted lease, inactive local admission,
post-install fence mismatch, and a real competing SQLite write transaction.
The busy-write oracle proves the durable row remains active after the failed
revoke, while both ordinary lease admission and recovery mutation routes remain
closed. Signed synthetic host acknowledgments prove gateway-side revocation
recording, fresh-epoch allocation, and prior stop preservation. The post-install
fault hook is compiled only for tests; there is no production fault route.

- `confirmed`: an authorized controlled test passed its stated oracle;
- `source-derived`: source/configuration directly establishes the claim;
- `inferred`: a documented consequence not yet exercised;
- `proposed`: future design input; and
- `unverified`: missing runtime or contract proof with a safe validation procedure.

A build, open socket, health response, or acknowledgment cannot upgrade game readiness, host
compatibility, isolation, authentication enforcement, or effect settlement. Runtime records must
include exact revision, contract versions/digests, instance/lease identities, clock/seed,
disposable fixture status, sanitized logs, and cleanup result.

## Spawned-process gateway checks

`crates/gateway/tests/gateway_real_process_episode.rs` spawns the built
`sts2-gateway-runtime` binary with `std::process::Command::new`, clears its
environment, and drives it over real loopback sockets. Because the decisions are
made by the served process — its listener, request parser, authorization policy,
durable SQLite store, and signed host sideband — this is the **spawned-process**
evidence class rather than an in-process library call.

The suite covers the repeated-episode lease profile (ADR 0033). One test
completes two consecutive episodes on one deployment and asserts that episode two
shares episode one's boot authority, lands on a distinct lease identity and a
strictly higher epoch, cannot dispatch episode one's receipt or resolve it under
an episode-two context, and leaves the fenced episode-one headers refused. It
also asserts the fail-closed default: a header-less completion stays permanently
revoking, and the next acquisition is refused with `lease_context_revoked`. The
second test restarts the process on the same durable store and asserts that the
boot authority rotates, no earlier epoch is reused, and the restarted process —
which never negotiated the profile — reports no witness and reopens nothing.

The signed host fake terminates the real `POST /api/v1/runtime/recovery` hop and
reproduces the HCJ1 canonicalization and HMAC-SHA256 acknowledgment proof, so the
observed request sequence is what the served gateway actually sent. This remains
component evidence: no native game, provider, deployment, or 24-hour soak is
established, and the downstream soak in `ascension-watchdog#58` stays separate.

The profile is gateway-local process state and is deliberately not durable, so a
restart discards it. Any operator procedure that relies on repeated episodes must
account for that single-process lifetime limit.

## Runtime adapter checks

The standalone runtime binary has bounded HTTP parser tests and builds with the pinned Rust
toolchain. Its v1 lane can run against a disposable synthetic downstream. Runtime-v2 route parsing,
envelope validation, ledger calls, error mapping, and fixed TCP forwarding are exercised against
synthetic peers. This is not a live host adapter test; host mutation and settlement remain unverified
and require a separately authorized downstream contract and disposable host environment.

The save-profile component tests use bounded in-memory provisioning and ledger ports plus an
ephemeral synthetic loopback peer. They cover fresh opaque identities, portable descriptors without
paths, injected unknown/foreign-content, traversal, symlink, and overwrite classifications, fixed
list/current/select/create-disposable/lookup mapping, closed mutation bodies, active-run, missing
active-run source, missing durable-intent, and stale-lease rejection before forwarding, duplicate
selection under a distinct operation ID, caller disconnect, timeout-after-write,
unknown-to-created reconciliation, and retained operator guidance. Creation receipts must echo the
reserved operation identity and the approved launch contract, or the gateway records an invalid
response and retains the operation as unknown.

The filesystem-refusal tests inject the adapter's inspection classification or typed port error
(`Traversal`, `SymlinkEscape`, `UnknownContents`, `ExistingContents`, or `Unavailable`) instead of
touching a real root, so they prove the provisioner's refusal mapping and that no create call
follows a refused inspection. They do not traverse a real filesystem, resolve a real symlink, or
prove containment. The restart tests reopen a shared record store from a second ledger or
provisioner, which proves that retained accepted/unknown intent is not dispatched again and that
allocation identities are not reused; there is no production durable-store adapter yet.

The attached runtime composes no durable operation-intent store, isolated-allocation port, or
launch-profile binding port, so every mutation fails closed before provisioning or forwarding with
an explicit capability result (`save_profile_active_run_unavailable`,
`save_profile_persistence_unavailable`, or `save_profile_provisioning_unavailable`) and no
production in-memory substitute is created. Read routes still forward through the fixed loopback
targets. These are source/component outcomes; they do not prove a real isolated filesystem
allocation, production durability, issue #50 launch-profile wiring, game-mod readback, native save
compatibility, or cross-restart durability.

The authorized exact-host lane now confirms the managed mod listener, downstream forwarding,
lease fencing, a Godot main-thread callback, the bounded STS2 host effect, and reversible disposable
profile cleanup. Process supervision/restart, concurrency isolation, and gameplay mutation remain
`unverified`.

## Runtime-v3 and co-op checks

The source lane tests the six fixed Runtime-v3 route/method pairs, bounded request/response JSON,
and rejection of arbitrary paths. Co-op tests cover peer capacity, duplicate/local-role rejection,
generation disagreement, disconnect, missing local identity, and mutation suspension. Process
supervisor tests cover capacity, ownership, inspection, graceful/forced stop seams, and release of
owned handles. These tests do not replace an authorized live process or target-host trace.

## Runtime-v2 wire closure

`crates/gateway/tests/runtime_v2_wire_closure.rs` round-trips every copied golden message, removes
each of the six nullable envelope members individually, and verifies decoding rejection. It also
checks unknown envelope member rejection. This deterministic decoder evidence does not establish
host or downstream runtime compatibility.

## Exo component integration

The combined Exo/component adapter tests every v3 route with read, mutate and control credentials
and verifies the MCP-session fence on all six routes before request-body decoding. Existing
Runtime-v2 journal, queue, authentication and lease regressions remain in the same full suite.

## Accepted native co-op consumer checks

The `coop-native-v1` consumer copies the protocol schema, manifest, conformance case, producer
capture, and all seventeen golden envelopes. Its forwarder tests reject duplicate and unknown
members, mismatched identity or lease headers, wrong route kinds, stale settled effect lineage,
wrong recovery kinds, accepted/unknown receipt generation drift, and unbounded producer errors.
The loopback transport test also verifies that a recovered instance, lease ID, and epoch, rather
than stale process configuration, are sent to the mod boundary. Service tests exercise the observation and
local-action routes against synthetic loopback downstreams and assert the exact game-mod paths,
body, credential, and response bytes. These checks establish component serialization and gateway
transport behavior only; they do not establish a native STS2 host, peer convergence, or gameplay.

The gateway-local v1 peer-route tests additionally require an operator-configured peer token and
peer ID, pin that token to the current instance/session/lease/epoch, reject caller-selected peer
substitution and stale leases, retain exactly one pending original operation, reject duplicates,
and allow recovery only on the matching bound route and operation. A changed returned authority
or a returned `local` observation peer that differs from the configured canonical peer cannot
settle or clear the record. The tests use synthetic loopback responses and do not prove native
message delivery, host mutation, two-peer behavior, or live settlement.

## MCP-session configuration

A pure MCP-session configuration test covers the independent default, explicit override and invalid
values without changing process environment. Cross-process identity issuance remains unverified.

## Co-op synchronization

`coop_reports_tests` injects monotonic instants and proves startup missing state, convergence,
disagreement, disconnect, expiry at the exact lifetime boundary, recovery, bounded roster,
closed reports, and generation regression refusal. `service_coop_tests` exercises the actual
fixed handlers with complete lease/scope checks, a pinned schema/golden response, malformed
input and an unavailable downstream port. These routes must never access that port.

Verify all eight entries under `protocol-artifact/coop-synchronization-v1/SHA256SUMS`.
The coordinated MCP `coop_gateway_runtime` gate launches both real executables with separate
read/control credentials, verifies the full report lifecycle and lease rejection, and asserts
that a listening downstream trap received zero connections. It is run explicitly with this
exact built gateway passed in `STS2_COOP_GATEWAY_BINARY`; see the MCP testing guide and
the coordinated evidence record. It proves coordination transport, not native multiplayer.

## Seeded-run gateway checks

The `seeded-run-v1` source/component suite checks the copied schema, manifest, conformance case,
goldens, and checksum inventory, then exercises fixed start/reconciliation route parsing, selected
context and digest validation, lease/epoch/correlation fences, semantic operation idempotency,
accepted and unknown read-only reconciliation, and journal restart recovery. The forwarder tests
assert that only `/v2/seeded-run` and `/v2/seeded-operations/{operation_id}` reach the mod boundary.

These deterministic checks establish gateway ledger, journal, and forwarding behavior. They do not
start a native run, establish canonical seed readback or a `run_started` host witness, verify
profile/save isolation, or prove gameplay and release compatibility.

## Runtime-map visibility checks

The `runtime-map-v1` consumer at current gateway main
`2b44bf347f790509c9f13378c89719d09366d45b` verifies current protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404` and schema digest
`ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b` through the copied manifest,
schema, conformance case, and golden checksum inventory. Forwarder tests cover the exact GET-only
route, bodyless request, downstream path, response budget, provenance and digest, configured
identity and epoch/generation fences, bounded graph topology, visited position/history/terminal
references, and independent generation-bound action bindings. Invalid identity, stale generation,
unknown fields, duplicate graph members, cycles, invalid action-option IDs, and oversized responses
fail closed. Exact UTF-8 byte boundaries, C0/DEL/C1 controls, and timeout ordering are checked at
the forwarder and service boundary. Overlapping coordinates and disconnected visible components
remain accepted.

These are deterministic source/component checks. They do not establish a running game-mod, host map
freshness, map projection compatibility, visualizer behavior, or a gameplay navigation effect.

## Runtime-v4 expert rest-action checks

The candidate `runtime-v4-expert-rest-action-v1` consumer pins schema digest
`bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd` and checks its copied
`SHA256SUMS` inventory. Forwarder tests exercise fixed POST/GET route parsing, JSON content and
body bounds, authenticated identity/lease/epoch/correlation matching, status mapping, nested expert
observation identity, generation fences, option-specific effect witnesses, typed selector legal
actions, and a bounded selector-admission catalog retained across response observations. All 16
goldens are checked for dispatch and reconciliation, all 22 schema-valid mutation fixtures are
rejected, and both serialized Smith and Mend producer-shaped lifecycles are checked message by
message. Service tests verify exact downstream paths and malformed profile rejection before any
downstream response is accepted.

These tests establish only gateway source/component behavior for a candidate profile. The artifact
has no admitted consumers; native producer serialization, host legality and effects, MCP/harness
mapping, deployment, and live settlement remain unverified.

## Proposed retained receipt query checks

The proposed receipt-query route validates every frozen accepted, settled, rejected, and unknown
response golden, then rejects unsorted or foreign participants, numeric spelling and duplicate-key
changes, generation lineage changes, fresh-observation evidence, status/receipt mismatches, settled
generation regressions, action/effect mismatches, and response identity drift. The route test also
verifies JSON content type and active-lease admission before any downstream forwarding. These checks
establish only gateway source/component behavior; they do not establish a native producer, a live
retained receipt, or cross-consumer recovery.

## Public checkpoint reference route

The `checkpoint_reference` service tests call the production dispatcher and synthetic loopback
producer. They cover fixed path/identity forwarding, auth/scope/session/lease/epoch/correlation
rejection, revoked/shutdown admission, foreign and privileged response rejection, duplicate keys,
8192-byte limits and unavailable producers. See ADR 0023. This is component evidence only;
no native capture, durable receipt or restore is established.
