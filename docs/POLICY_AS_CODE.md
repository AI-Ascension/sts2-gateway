# Policy as code

## Source of truth

`policy.toml` is the target-local policy declaration. The Rust `repo-policy` tool under
`tools/repo-policy` is a small governance checker, not gateway product code. It checks the exact
target-relative required-file list, path-safe exemptions, source language, file budgets, Markdown
local links, MIT license/header rules, Rust workspace/toolchain declarations, and GitHub workflow
safety. It also checks Rust module reachability: a tracked `.rs` file that no crate root can reach
is reported, because losing a `mod` declaration is not a compile error anywhere — the file simply
stops being read, silently dropping live code and any test inside it.

The policy is deliberately stricter than a formatting check: workflows need explicit permissions,
must not use `pull_request_target`, `continue-on-error: true`, or `|| true`, and every external
action must be pinned to an immutable commit or digest. The target has no policy exemptions.

## Local entrypoint

Rust executable sources under `crates/gateway/src/bin/` are included in policy checks. The old
basename-wide `bin` exclusion and oversized service exemption have been removed; the attached
service is split into configuration, control, downstream, v2, v3, and test modules under the same
existing budgets. No new exception replaces that coverage.

Run from `sts2-gateway`:

```text
cargo run --locked --package repo-policy -- --strict
```

`--strict` promotes size warnings to failures. The same command runs in
[`.github/workflows/policy.yml`](../.github/workflows/policy.yml), after the tool's own tests. The
CI workflow repeats formatting, Clippy, and tests with the pinned lockfile.

## Review rules

Policy changes are code changes. Explain why a required path, ignored directory, size budget,
exemption, workflow permission, action pin, or language rule changes. Exemptions must identify one
exact repository-relative path and a durable provenance reason; copied implementation source never
qualifies. Do not add a policy exception to hide product behavior or a missing contract.

The checker is intentionally target-local so this repository remains reproducible when sibling
targets evolve. It does not inspect sibling trees, planning material, game files, process state,
provider state, or deployment systems.

The former directory-name ignore for `bin` also skipped `crates/gateway/src/bin`, including the
attached runtime's source. That ignore is removed: runtime source and its concern-specific test
modules now receive the unchanged size, license, and language checks. A regression test loads this
repository's actual policy and verifies the runtime entrypoint, service, and HTTP parser are scanned.

## Rust module reachability

`RUST002` reports a Rust file under a crate's `src`, `tests`, `benches`, or `examples` directory that
no crate root can reach. rustc compiles only files reachable from a crate root, so deleting a `mod`
item — which a rename or a conflict resolution does silently — drops the file, and every `#[test]`
inside it, from the build without any compiler diagnostic. The check follows rustc's resolution
rules: a crate root keeps its own directory for `mod` children, any other file resolves them beside
itself, `#[path = "..."]` resolves relative to the file carrying the attribute and gives the loaded
module its own directory, inline modules scope their children under their own name, and `include!`
shares the includer's child directory.

`cfg` and `cfg_attr` gating is deliberately ignored: a module compiled out on one platform is still
reachable, so every branch of a `cfg_attr`-selected `#[path]` counts. The check must never report a
reachable file, because a false finding would block a legitimate build; the unit tests build real
crates on disk to pin each resolution rule, including the `r#`-prefixed module names and the
attribute-before-visibility form `#[path = "x.rs"] pub(super) mod x;`.

The remedy for a finding is one of two explicit decisions, never a silent deletion: restore the
declaration when the file is live code whose wiring was lost, or delete the file when it is a
superseded duplicate. `crates/gateway/src/bin/runtime_support/service_config_identity.rs` was the
second case: its `configured_mcp_session` was byte-identical to the live `service_config_values.rs`
copy that `service_config.rs` re-exports and calls, so wiring it back would have been an immediate
`E0592 duplicate definitions`.
