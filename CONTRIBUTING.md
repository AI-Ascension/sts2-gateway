# Contributing to sts2-gateway

## Scope

Contributions must keep the gateway as the lifecycle and routing control plane. A change belongs
here only when it concerns instance identity, process ownership, leases, fencing, authentication,
fixed route selection, readiness/health, isolation, capacity, cleanup, or gateway-owned diagnostics.
Game rules, host access, MCP semantics, model/provider behavior, and harness artifacts belong to
their owning targets.

## Before a change

1. Read [AGENTS.md](AGENTS.md) and the applicable target documents.
2. Identify the project-owned requirement, decision, consumer, compatibility version, and oracle.
3. Inspect the target tree and preserve unrelated user files.
4. Do not use live game processes, valued saves, providers, credentials, or remote listeners.

Public lifecycle and data-plane behavior must define identity namespaces, state transitions, stable
errors, deadlines, retryability, idempotency, cancellation, and privacy impact before implementation.
A missing decision is a blocker, not permission to invent a route or recovery behavior.

## Implementation expectations

Use small cohesive Rust modules and explicit ports for clock, scheduler, process, transport, and
storage where needed. Keep queues and payloads bounded, lock ordering documented, and shutdown
joinable. Validate caller, instance, session, lease, epoch, route, method, headers, and body before
forwarding. Do not make a generic proxy or retry an admitted mutation without an idempotency oracle.

Use deterministic fake instances and injected time for lifecycle tests. A caller timeout or disconnect
must lead to an explicit status/reconcile result. A crash or mismatch must not reroute work to a
different instance.

## Validation

Run the exact local gates from [POLICY_AS_CODE.md](docs/POLICY_AS_CODE.md) and
[TESTING.md](docs/TESTING.md). At minimum:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

Record command results, toolchain, changed contracts, evidence level, and skipped runtime checks.
Do not conceal failures with retries, `|| true`, ignored results, or non-blocking workflow steps.

## Review and release

Pull requests should explain ownership, dependency direction, security impact, compatibility
classification, tests, documentation, and follow-up work. Update [CHANGELOG.md](CHANGELOG.md) for
user-visible contract changes. Follow [RELEASING.md](RELEASING.md); contribution preparation does
not authorize commits, pushes, tags, publication, deployment, or live-game validation.
