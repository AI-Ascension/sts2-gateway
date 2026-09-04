# Policy as code

## Source of truth

`policy.toml` is the target-local policy declaration. The Rust `repo-policy` tool under
`tools/repo-policy` is a small governance checker, not gateway product code. It checks the exact
target-relative required-file list, path-safe exemptions, source language, file budgets, Markdown
local links, MIT license/header rules, Rust workspace/toolchain declarations, and GitHub workflow
safety.

The policy is deliberately stricter than a formatting check: workflows need explicit permissions,
must not use `pull_request_target`, `continue-on-error: true`, or `|| true`, and every external
action must be pinned to an immutable commit or digest. The target has no policy exemptions.

## Local entrypoint

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
