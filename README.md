<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-dark.svg">
  <img alt="AI-Ascension — Inspect how AI requests to a game get fenced, one Rust contract at a time. Bounded runtime host trace confirmed. Deterministic tests: confirmed." src="https://raw.githubusercontent.com/AI-Ascension/.github/main/profile/assets/banner-light.svg" width="100%">
</picture>

# sts2-gateway

> **AI-Ascension · tier 2: control plane · home of the public proof** — In-memory control plane for game-host instances: lifecycle, one lease per instance with epoch fencing, and fixed routes.
>
> **Status:** deterministic tests and one bounded attached-host runtime trace `confirmed` for STS2 v0.107.1 on Windows x86-64 · general lifecycle and broader compatibility `unverified`.
> **Proof:** [45-second browser replay](https://ai-ascension.github.io/proof.html) · [Evidence ledger](https://ai-ascension.github.io/evidence.html) · [This repository on the map](https://ai-ascension.github.io/repositories.html#sts2-gateway)
> **Proof source:** [crates/gateway/tests/control_plane.rs](crates/gateway/tests/control_plane.rs) — the replay mirrors these tests.
> **Owner:** The gateway boundary owner is responsible for the lifecycle and routing control plane: instance records, leases and lease epochs, fencing, fixed forwarding policy, and cleanup.
> **Contribute:** [Organization guide](https://github.com/AI-Ascension/.github/blob/main/CONTRIBUTING.md) · [First tasks](https://ai-ascension.github.io/contributing.html)
>
> AI-Ascension is an independent project. It is not affiliated with or endorsed by Mega Crit or Valve and grants no rights to game files, assets, or marks.

Status: Wave 2 POC plus bounded runtime-adapter proof. The target-owned gateway package provides a
deterministic control-plane core and injected boundary ports; the separate runtime binary adds one
attached loopback lane. An authorized trace now confirms that lane through the exact recorded host;
generic process supervision, multi-instance lifecycle, and broader compatibility remain outside it.

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
credentials, arbitrary proxying, or implicit remote discovery. It consumes only an inert copied
`sts2-protocol/poc-v1` artifact for this proof. A forwarded request must have a validated instance,
session, lease, lease epoch, route, method, and bounded body; listener reachability is not
authentication.

The POC test allocates and readies fake instances, forwards a fixed command route, and proves that
stale epochs and a proof from another instance are rejected before transport. It is a gateway
control-plane test, not evidence that a game-mod process is running or that an action settled.

## Evidence and provenance

This target is intentionally source/test bounded. The project policy and target decisions are
normative for this repository. Existing planning material is used only as a structural and
documentation exemplar. Planning and retained evidence are inputs labelled `proposed`, `inferred`, or
`unverified` unless a controlled test establishes otherwise. No reference implementation source,
proprietary game file, save, provider credential, or generated product output is copied here. The
protocol artifact is copied as explicit release-like data only.

The current state is `source-derived` from this tree and its policy files, with `confirmed`
deterministic fake-instance outcomes, a controlled component lane, and one authorized exact-host
runtime trace. Generic process startup/supervision, isolation under real concurrency, restart
reconciliation, and release behavior remain `unverified`. See the [compatibility
record](docs/COMPATIBILITY.md) and [testing plan](docs/TESTING.md).

## Local validation

Run these commands from this directory:

```text
cargo run --locked --package repo-policy -- --strict
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
(cd protocol-artifact/poc-v1 && sha256sum -c SHA256SUMS)
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
- The staged gateway investigation prompt is a discovery input, not an implementation or runtime
proof; it is maintained outside and is not copied into this repository.

## Attached runtime slice

The target now includes the standalone `sts2-gateway-runtime` binary. It is a bounded, authenticated
single-instance adapter for the first runtime slice: it exposes allocation, readiness, state, action,
and release routes on loopback, validates the configured identity/lease/epoch/correlation fence, and
forwards only the fixed runtime paths to an already attached mod listener. The MCP process reaches
this binary over its real TCP adapter; the gateway does not accept arbitrary paths or headers.

This binary does not launch or supervise a game process in this sprint. Its fixed instance and
attached downstream configuration are intentional for the vertical slice; the exact-host forwarding
and lease path is confirmed in the dated authorized trace. General process lifecycle, multi-instance
scheduling, restart reconciliation, and graceful shutdown remain `unverified`.
