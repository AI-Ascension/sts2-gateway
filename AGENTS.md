# Target Instructions for Coding Agents

## Scope and authority

These instructions apply to `sts2-gateway`. Follow the caller's direct instructions first, then
these rules. The detailed target policy is in [architecture](docs/ARCHITECTURE.md),
[product](docs/PRODUCT.md), [coding standards](docs/CODING_STANDARDS.md),
[testing](docs/TESTING.md), [compatibility](docs/COMPATIBILITY.md),
[licensing](docs/LICENSING.md), [workflows](docs/WORKFLOWS.md),
[policy as code](docs/POLICY_AS_CODE.md), and [release procedure](RELEASING.md).

The gateway is an original project boundary. The planning snapshot supplies standards; the
`sts2-harness-rust` tree supplies structure and documentation examples only. Do not copy, vendor,
transliterate, or cite another implementation as a product design. Retained evidence and proposed
planning material must keep their evidence labels.

## Ownership invariants

- Gateway owns instance identity, process ownership, lifecycle, readiness, health, authentication,
  authorization, leases, epochs, fencing, fixed routes, isolation, capacity, and cleanup.
- Game rules, semantic game state, host objects, loader/ABI translation, and mutation authority stay
  in the game-core/game-mod boundaries.
- MCP framing, tool catalogs, role mapping, and MCP-facing errors stay in `sts2-mcp-server`.
- Harness scheduling, model/provider calls, episodes, replay, scoring, and artifacts stay in
  `sts2-harness`.
- `sts2-protocol` may be used only for an accepted, language-neutral, transport-neutral contract with
  named consumers, versioning, and conformance. It is not a generic common-code destination.
- Runtime communication and compile-time dependency graphs must remain separate and acyclic.

Every future forwarded request must validate caller identity, session, instance, lease, lease epoch,
correlation identity, fixed method/path/body limits, and the target's current admission state. Never
forward arbitrary paths or headers, select a fallback instance after a mismatch, or treat network
reachability as authentication. A stale lease or epoch fails closed before forwarding.

Use injected clock, process, readiness, transport, and lease-decision ports. Bound queues and payloads. Preserve an
accepted operation after caller timeout or disconnect as a status/reconciliation outcome; do not
blindly retry a mutation. A crash quarantines only its instance. Shutdown closes admission, settles
or marks outstanding work, revokes leases, and joins owned tasks.

## Preparation and safety

The Wave 1 foundation was governance-only. This Wave 2 initialization may add one non-empty,
target-owned control-plane package and deterministic fakes in its tests, but not concrete product
adapters, game behavior, product crates outside that package, or empty placeholder crates. Do not
launch or kill processes, expose listeners, contact providers, access game files, use saves, install
a mod, or retain credentials. Keep fakes bounded and clearly test-only.

Preserve unrelated files and existing experiments in sibling targets. Do not initialize Git, change
history, publish, deploy, or modify any path outside this target when working as gateway owner.

## Required validation

From the target root, run:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

Report unavailable or skipped commands as `unverified`, including the missing precondition. A
passing policy or compile check is not proof of process, host, route, security, isolation, or
release behavior.

## Documentation and review

Before adding a public route, field, state, error, identity, lease operation, or timing rule, write
the requirement, owner, compatibility classification, rejection/cancellation behavior, and
deterministic oracle. Add an architecture decision record when ownership, dependency direction,
trust, or public contract changes. Update the changelog and affected docs in the same change.

Do not use `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` for ordinary input or
lifecycle failures. Keep unsafe code forbidden unless an approved boundary decision explicitly
requires it; this target has no host/FFI exception in the foundation.
