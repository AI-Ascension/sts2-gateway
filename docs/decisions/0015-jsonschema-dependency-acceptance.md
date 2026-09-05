# ADR 0015: Acceptance of the `jsonschema` product dependency

- Status: Accepted (owner decision delegated 2026-09-05, "resolve these issues"; recorded by the
  review lane)
- Date: 2026-09-05

## Context

Gateway PR #7 (merged as `52bd147667667d62d8d10c7de861d996f365600b`) introduced
`jsonschema = "=0.52.1"` in `crates/gateway/Cargo.toml` to validate Runtime-v3 gameplay envelopes
against the embedded canonical schema (ADR 0014). It is the first product dependency beyond
`serde`/`serde_json`, and it grows the locked workspace graph from 22 to 88 packages. ADR 0001
permits owner-local and accepted neutral-contract dependencies but did not record an explicit
acceptance for this tree; the Wave 1 review (item P7-1) required one.

## Decision

The dependency is accepted for the attached Runtime-v3 adapter under these conditions, each of
which is verified in this record and must hold for every future bump:

1. **Pinned exactly.** `jsonschema = { version = "=0.52.1", default-features = false }`;
   `Cargo.lock` carries checksum `93e842fa72fd1e50ca4676a527641c13f5ee0d423ac699bfe1cd2afa3a4fdbac`.
   `confirmed` (manifest and lockfile at `52bd147`).
2. **Default features off.** `cargo tree --locked -p sts2-gateway -e normal --format "{p} [{f}]"`
   reports `jsonschema v0.52.1 []`: no `resolve-http`, `resolve-file`, or `resolve-async`
   feature, so the validator never fetches remote or filesystem references. `confirmed`.
3. **Notice and license.** `THIRD_PARTY_NOTICES.md` names `jsonschema` 0.52.1 (MIT, per
   `cargo metadata`) and states that the transitive set is enumerated in `Cargo.lock`. `confirmed`.
4. **`unsafe_code = "forbid"` still holds.** The workspace lint is unchanged for every workspace
   crate; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes.
   The lint governs first-party source only; third-party crates in the tree are accepted as
   audited upstream code, pinned by checksum, and nothing in the enabled feature set turns on a
   network, filesystem, or FFI path. `confirmed` for the lint; `source-derived` for the feature
   inspection.
5. **Built once, failure surfaced.** The validator is compiled once in a `OnceLock` in
   `runtime_v3_gameplay_forwarder.rs::validate_envelope`. If the embedded schema failed to
   compile, the forwarder would reject every Runtime-v3 envelope (fail closed) without a startup
   error. To make that failure visible, the unit test
   `embedded_runtime_v3_schema_compiles_and_admits_golden_request` builds the validator from the
   embedded schema and admits a golden request through `validate_envelope`; CI runs it on every
   push. `confirmed` (local and CI runs cited in the merging PR).
6. **Documented.** `docs/COMPATIBILITY.md` and `docs/ARCHITECTURE.md` state that Runtime-v3
   envelope validation depends on this crate and that `unsafe_code = "forbid"` remains in force.

## Consequences

- Bumping `jsonschema` or enabling any of its features requires re-running the checks above,
  updating the notice, and amending this ADR with a dated entry.
- The transitive set (74 packages in the gateway's locked normal dependency tree, including
  `regex`, `fancy-regex`, `ahash`, `parking_lot`, `getrandom`, `uuid-simd`) is accepted as part of
  the pinned lockfile; Dependabot proposals must not be merged without the same review.
- The dependency does not change any wire byte, route, or artifact digest; Runtime-v3 live
  behavior remains `unverified` as recorded in ADR 0014 and `docs/COMPATIBILITY.md`.

## Alternatives considered

1. Hand-rolled structural validation (the earlier bounded lane in PR #6): rejected because it
   cannot prove conformance to the canonical schema bytes that CI pins by digest.
2. Vendoring a minimal validator: rejected because it copies third-party implementation source
   into a repository that forbids transliterated implementations.
