# Gateway testing and evidence

## Current status

The target contains the gateway control-plane package and repository tooling. Policy, formatting,
lint, build, and package tests run without a product workspace. The package tests use deterministic
fake clock, process, readiness, transport, and lease-decision seams. No gateway listener, child
process, game host, provider, save, or deployment is used by these checks.

## Baseline commands

Run from the target root:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The policy command comes first in the normal local sequence. CI runs the same commands with bounded
job timeouts. A missing toolchain or unavailable dependency is reported as unverified; it is not
converted into a passing skip. The package suite confirms only in-memory control-plane outcomes,
not runtime compatibility.

The current deterministic suite covers allocation and readiness, process inspection and crash
failure, lease expiry and forced cleanup, stale epoch and wrong-instance rejection before transport,
graceful release, shutdown admission closure, bounded fixed-route forwarding, and transport/stop/
start failure reporting. The POC case additionally verifies the copied artifact identity while
combining readiness, fixed command forwarding, stale-epoch rejection, and wrong-instance fencing.
The fakes do not represent live process or network behavior.

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

## Evidence levels

- `confirmed`: an authorized controlled test passed its stated oracle;
- `statically derived`: source/configuration directly establishes the claim;
- `inferred`: a documented consequence not yet exercised;
- `proposed`: future design input; and
- `unverified`: missing runtime or contract proof with a safe validation procedure.

A build, open socket, health response, or acknowledgment cannot upgrade game readiness, host
compatibility, isolation, authentication enforcement, or effect settlement. Runtime records must
include exact revision, contract versions/digests, instance/lease identities, clock/seed,
disposable fixture status, sanitized logs, and cleanup result.
