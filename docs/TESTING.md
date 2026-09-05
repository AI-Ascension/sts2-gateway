# Gateway testing and evidence

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
The auth component additionally covers expired credentials, route scopes, and previous-token
rotation overlap; these tests use an injected test time and do not prove an external issuer or live
secret-management system. The attached runtime also tests that a mismatched configured MCP-session
header fails at the lease fence before downstream forwarding.
The control-plane fakes do not represent real processes or game hosts. Ephemeral TCP tests exercise
actual socket framing, timeouts, and forwarding only against synthetic peers.

Late retained settlement is checked after a newer authoritative observation: the historical receipt
remains replayable, but cannot rewind admission generation. Both accepted and unknown operations
are covered. State refresh rejects a regressed generation, and restore rejects a settled receipt
without a successor or a persisted binding older than a retained result.
The attached Runtime-v2 action profile also rejects operation IDs that cannot be reconciled by the fixed
single-segment receipt route; an ephemeral listener verifies these invalid IDs cause zero forwards.
Release/shutdown followed by allocation is rejected and leaves old lease-protected requests fenced.

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

- `confirmed`: an authorized controlled test passed its stated oracle;
- `source-derived`: source/configuration directly establishes the claim;
- `inferred`: a documented consequence not yet exercised;
- `proposed`: future design input; and
- `unverified`: missing runtime or contract proof with a safe validation procedure.

A build, open socket, health response, or acknowledgment cannot upgrade game readiness, host
compatibility, isolation, authentication enforcement, or effect settlement. Runtime records must
include exact revision, contract versions/digests, instance/lease identities, clock/seed,
disposable fixture status, sanitized logs, and cleanup result.

## Runtime adapter checks

The standalone runtime binary has bounded HTTP parser tests and builds with the pinned Rust
toolchain. Its v1 lane can run against a disposable synthetic downstream. Runtime-v2 route parsing,
envelope validation, ledger calls, error mapping, and fixed TCP forwarding are exercised against
synthetic peers. This is not a live host adapter test; host mutation and settlement remain unverified
and require a separately authorized downstream contract and disposable host environment.

The authorized exact-host lane now confirms the managed mod listener, downstream forwarding,
lease fencing, a Godot main-thread callback, the bounded STS2 host effect, and reversible disposable
profile cleanup. Process supervision/restart, concurrency isolation, and gameplay mutation remain
`unverified`.

## Runtime-v2 wire closure

`crates/gateway/tests/runtime_v2_wire_closure.rs` round-trips every copied golden message, removes
each of the six nullable envelope members individually, and verifies decoding rejection. It also
checks unknown envelope member rejection. This deterministic decoder evidence does not establish
host or downstream runtime compatibility.
