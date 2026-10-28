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

## 2026-10-28: 0.60.1 dependency candidate (PR #148)

- The exact direct pin moves to `=0.60.1`; its Cargo.lock checksum is
  `65de752665c4a06528af6759955eecf60ba73ae7442d35349963a09c8031cd71`.
  `jsonschema-regex`, `jsonschema-value`, and `referencing` resolve to 0.60.2;
  their exact versions and checksums remain recorded in Cargo.lock.
- `default-features = false` and the first-party `unsafe_code = "forbid"`
  setting remain unchanged (source-derived). The notice now identifies 0.60.1.
- Hosted CI for head `3a807aed5302ecd75afae92156502116f24dd4e0`, composed
  with main `3dbf877c881bd4fadae03dea0b416b624ffec575` as `640575f702b6268e9cb7fec9ae0b9534c01db213`,
  passed formatting, all-feature Clippy, documentation lints, copied protocol
  artifact checksum inventories, and 660 tests with zero failures or ignored
  tests. This includes `embedded_runtime_v3_schema_compiles_and_admits_golden_request`.
  Evidence: [Rust job 122937171785](https://github.com/AI-Ascension/sts2-gateway/actions/runs/40722585251/job/122937171785).
- The locked normal feature graph and exact-version registry license metadata
  were reviewed against the exact manifests, lockfile and crates.io metadata.
  The only reverse dependency of jsonschema in the lockfile is sts2-gateway;
  its dependency disables defaults and selects no features. Both workspace
  member manifests and the workspace feature declarations were inspected:
  no other edge enables jsonschema features. The registry feature map for
  0.60.1 places resolve-http, resolve-file and TLS in default features, while
  resolve-async is opt-in. None is selected by this graph (source-derived).
  No reqwest, rustls or tokio package appears in the lockfile. This is static
  graph evidence, not a claim that cargo tree was executed locally.
  Exact-version registry metadata identifies jsonschema 0.60.1 and
  jsonschema-regex/jsonschema-value/referencing 0.60.2 as MIT, unyanked,
  Rust 1.85.0 minimum; all four registry checksums match Cargo.lock.
  No tarball-byte or complete advisory audit is claimed. Re-run all required
  hosted gates after candidate edits; a packaged release remains out of scope.
- This records source/component evidence only. It does not qualify native
  host behavior, actual provider integration, deployment, or a packaged release.
