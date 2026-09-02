<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-dark.svg">
  <img alt="AI-Ascension — Inspect how AI requests to a game get fenced, one Rust contract at a time. Runtime: unverified. Deterministic tests: confirmed." src="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-light.svg" width="100%">
</picture>

# sts2-gateway

> **AI-Ascension · tier 2: control plane · home of the public proof** — In-memory control plane for game-host instances: lifecycle, one lease per instance with epoch fencing, and fixed routes.
>
> **Status:** deterministic in-memory tests `confirmed` at the pinned commit · runtime, host, and game compatibility `unverified` · nothing is live.
> **Proof:** [45-second browser replay](https://ai-ascension.github.io/proof.html) · [Evidence ledger](https://ai-ascension.github.io/evidence.html) · [This repository on the map](https://ai-ascension.github.io/repositories.html#sts2-gateway)
> **Proof source:** [crates/gateway/tests/control_plane.rs](crates/gateway/tests/control_plane.rs) — the replay mirrors these tests.
> **Owner:** The gateway boundary owner is responsible for the lifecycle and routing control plane: instance records, leases and lease epochs, fencing, fixed forwarding policy, and cleanup.
> **Contribute:** [Organization guide](https://github.com/AI-Ascension/.github/blob/main/CONTRIBUTING.md) · [First tasks](https://ai-ascension.github.io/contributing.html)
>
> AI-Ascension is an independent project. It is not affiliated with or endorsed by Mega Crit or Valve and grants no rights to game files, assets, or marks.

Status: Wave 2 codebase initialization. The target-owned gateway package provides a deterministic
control-plane core and injected boundary ports; no listener, concrete process supervisor, game
connection, or live runtime claim exists in this target.

## Owner and boundary

The `sts2-gateway` boundary owner is responsible for the external lifecycle and routing control
plane: instance records, allocation, process ownership, readiness and health observation,
authentication and authorization, leases and lease epochs, fencing, fixed forwarding policy,
per-instance isolation, bounded backpressure, and cleanup.

Its intended consumers are the harness control coordinator and the thin MCP server adapter. The
gateway may address an isolated `sts2-game-mod` process at runtime, but it does not import or own
host, loader, game-rule, or game-state implementation. Operator and recovery clients are separate
control-plane consumers. Runtime communication and compile-time dependencies are distinct; see
the [architecture](docs/ARCHITECTURE.md).

The gateway does not own game rules, host objects, managed loader code, MCP semantics or tool
catalogs, model/provider execution, harness episodes or artifacts, direct game files, saves,
credentials, arbitrary proxying, or implicit remote discovery. A forwarded request must have a
validated instance, session, lease, lease epoch, route, method, and bounded body; listener
reachability is not authentication.

## Evidence and provenance

This target is intentionally initialization-only. The project policy and target decisions are
normative for this repository. `sts2-harness-rust` is used only as a structural and documentation
exemplar. Planning and retained evidence are inputs labelled `proposed`, `inferred`, or
`unverified` unless a controlled test establishes otherwise. No reference implementation source,
proprietary game file, save, provider credential, or generated product output is copied here.

The current state is `statically derived` from this tree and its policy files, with `confirmed`
deterministic fake-instance unit/integration outcomes limited to the package's in-memory seams.
Concrete process startup, health/readiness transport, route compatibility, authentication at an
external boundary, isolation under real concurrency, host compatibility, and release behavior
remain `unverified` until an authorized disposable test exists. See the [compatibility
record](docs/COMPATIBILITY.md) and [testing plan](docs/TESTING.md).

## Local validation

Run these commands from this directory:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The first command is the local policy entrypoint and checks required paths, licenses, links,
workflow restrictions, Rust configuration, language restrictions, and file budgets. The package
tests exercise only injected deterministic fakes; these commands do not launch a gateway listener,
game process, MCP server, provider, or host.

## Repository map

- [AGENTS.md](AGENTS.md) is the target operating contract.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) defines ownership, trust, and dependency direction.
- [docs/PRODUCT.md](docs/PRODUCT.md) defines the gateway product boundary and non-goals.
- [docs/REPOSITORY_LAYOUT.md](docs/REPOSITORY_LAYOUT.md) records the package and boundary layout.
- [docs/TESTING.md](docs/TESTING.md) records the deterministic package suite and future security tests.
- [docs/decisions/0001-gateway-ownership-and-dependencies.md](docs/decisions/0001-gateway-ownership-and-dependencies.md)
  records gateway ownership and dependency rules.
- [docs/decisions/0002-sixth-target-protocol-boundary.md](docs/decisions/0002-sixth-target-protocol-boundary.md)
  records the current sixth-target protocol decision.
- The staged [gateway investigation prompt](../planning/prompt-corpus/staged/sts2-gateway-INVESTIGATION_PROMPT.md)
  is a discovery input, not an implementation or runtime proof.
