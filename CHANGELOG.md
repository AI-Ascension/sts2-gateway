# Changelog

All notable changes to `sts2-gateway` are recorded here. Component evidence is separate from
host compatibility and release publication.

## [Unreleased]

- Name the rejected header in the `unsupported_header` refusal, additively and by name only. Refs #541.

- Correct the "pinned by tests" list in `docs/POLICY_AS_CODE.md`: it still read *three* consequences after
  the inline-`mod` ownership rule became the fourth, so a reader counting the registry would miss a rule
  `RUST002` now follows. Prose only; the rule, its tests, and every other check are unchanged. Closes #110.

- Stop `RUST002` reporting module files that `rustc` compiles. An inline `mod` block owns no file at all,
  `#[path]` or not — the value names the *directory* its children live in — so a declaration inside one must
  resolve against the directory its enclosing blocks have folded to, not the carrying file's own. The old
  arm joined the file's directory instead, which reported live files and missed orphans wherever a `#[path]`
  sat inside an inline block. `rustc` 1.97.1 dep-info pins the folds: `mod a { #[path = "x.rs"] pub mod m; }`
  reads `src/a/x.rs`, and a `#[path]` inside a `#[path]`-named block reads `src/thread/other.rs` rather than
  `src/other.rs`. Differential over 21 shapes: 13 rule errors before, 12 fixed, 0 regressions, the remainder
  the documented `cfg_attr` both-branches policy. Five regression tests fail on the previous revision and
  pass here. Latent in this tree (no inline `#[path]` site); `sts2-harness#499` vendored the same fold and
  `#501` tracks its own coverage gap. Policy only; no gateway behavior, contract, or native effect. Closes
  #105.

- Deny `rustdoc::private_intra_doc_links` and `rustdoc::redundant_explicit_links` in the doc gate,
  and repair the one link they were passing over. The gate denied only
  `rustdoc::broken_intra_doc_links`, so it exited 0 and reported success while printing a
  `private_intra_doc_links` warning on `bind_attached_lease`: the doc comment linked
  `[Self::authenticate]`, which resolves only because the gate itself always passes
  `--document-private-items` and therefore breaks for every reader who does not.
  `crates/gateway/src/process_lifecycle.rs:115` now keeps the prose with a code span instead of the
  link, the same remedy `sts2-harness#492` used for the equivalent sites. Non-vacuity was measured,
  not assumed: with the link re-introduced the tightened gate aborts on the private item and exits
  101, and with the fix in place the same flags exit 0 with zero warnings. Policy and documentation
  only; no gateway behavior, contract, or native effect. Closes #100.

- Stop `RUST002` reporting four rustc-valid module shapes as unreachable, since a false finding
  invites deleting a file the build needs. The declaration parser now reads raw identifiers through
  their ordinary name (`mod r#move;` loads `move.rs`, and an inline `mod r#type { ... }` nests in the
  unprefixed `type/` directory), keeps every attribute group directly above the item so an unrelated
  `#[allow(dead_code)]` or a second `#[cfg_attr(..., path = "...")]` branch cannot hide the honoured
  `#[path]`, and compares reached files against the reduced spelling the file walk produces so a
  `#[path = "../other/y.rs"]` value that escapes its own directory is not mistaken for an orphan. Five
  regression tests pin the four shapes plus the counter-intuitive unprefixed-directory rule; each one
  was confirmed failing before the fix. Found by review of #99; it blocked porting the rule to the six
  sibling repositories that vendor this tool. Policy and documentation only; no gateway behavior,
  contract, or native effect. Closes #102.

- Retire the gateway's undeclared, never-compiled `service_config_identity.rs` and gate the class
  durably. `crates/gateway/src/bin/runtime_support/service_config_identity.rs` defined
  `configured_mcp_session`, but no `mod`, `#[path]`, `include!`, or manifest target reached it, so
  rustc never compiled or type-checked it while it sat beside its declared siblings. Its body
  duplicated the live `service_config_values.rs` copy, which `service_config.rs` re-exports and calls
  on the `STS2_MCP_SESSION_ID` admission path; that surviving copy is confirmed authoritative and the
  orphan is deleted. The durable half is `RUST002` in `repo-policy`: a module-reachability check that
  mirrors rustc's filename resolution — `mod x;` to `DIR/x.rs`/`DIR/x/mod.rs`, crate roots from
  `[lib]`/`[[bin]]` and the `src/bin`/`tests`/`examples`/`benches` target conventions, `#[path]`- and
  `include!`-loaded files keeping their children in their own directory, `#[cfg_attr]` paths,
  `mod.rs` child lookup, and one directory level per inline `mod` block — and fails `--strict` when a
  tracked `.rs` file in a compiled package is reachable from no crate root. Unit tests cover an
  orphaned file, the repository's own `#[path]` siblings and `[[bin]] path` targets, and the real
  tree. Policy and documentation only; no gateway behavior, contract, or native effect. Closes #98.

- Repair the gateway crate's one dangling intra-doc link and gate the class durably. The `host_lease`
  child of `recovery::recovery_store` linked a bare `[`RecoveryLease`]`, but a bare link resolves only
  against the file's own scope: the parent module's private `use super::recovery_types::{…}` does not
  count, and this file's own import list omits the type, so the link resolved to nothing while the
  crate still built, linted and tested green. `RecoveryLease` is real — it is the `pub struct` at
  `recovery_types.rs:164` — so this was a pure scope/path defect, fixed by qualifying the link to
  `super::super::recovery_types::RecoveryLease` (the same pattern `sts2-harness#474` used for the
  identical class). The durable half is a `cargo doc` step in the `rust` job with
  `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links --document-private-items"`, because a default
  rustdoc run skips this private module entirely; no workflow had run `cargo doc`/`rustdoc` here, so
  nothing owned the class. Documentation and gate only; no code, contract or native effect. Refs #96.

- Add the missing gateway hop for the whole-manifest read. `sts2-game-mod` already serializes the
  complete `game-information-content-manifest-v1` catalog and serves it from its fixed owner route,
  but the gateway and MCP hops were absent, so no authenticated, fenced surface exposed the manifest
  at all — the blocker this change removes. `GET /v1/instances/{instance_id}/game-information/content-manifest`
  is now admitted as a bodyless `Read` and forwarded only to the fixed
  `GET /api/v1/game-information/content-manifest`, reusing the existing game-information
  authorization, instance fence, and caller-disconnect cancellation because those paths already key
  off the shared route parse. The pinned profile permits a 16 MiB message while this route's framing
  ceiling is 128 KiB, so the route advertises its own smaller bound and refuses a larger declared
  exchange from the declaration alone with the pinned profile's own
  `result_limit_exceeded`/`serialized_payload_too_large` arm: a prefix of a catalog is a different,
  plausible catalog, so the manifest is never shortened. Responses are validated for strict JSON
  without duplicate keys, protocol version, schema digest, exact provenance, the pinned schema
  itself (whose copied bytes are re-hashed against the pin before use), correlation equality, kind,
  and kind/status pairing; a configured 64-hex `STS2_GAME_INFORMATION_CONTENT_MANIFEST_ID` also
  fences `inventory_revision` with `409 game_information_content_manifest_scope_rejected`, while a
  non-digest label pins nothing. A body-borne `GET`, a missing or out-of-alphabet correlation, and
  every lease, authority, or contract failure fail closed before or instead of relaying catalog
  bytes, and a typed producer error keeps its status and body without fabricating a manifest. This
  slice deliberately adds no operator enablement flag and no negotiated-capabilities offer, because
  those closed artifacts' `operation` enums would have to change. See
  [ADR 0036](docs/decisions/0036-game-information-content-manifest-route.md); the producer's own
  side is `sts2-game-mod#83`. Additive only: no existing route, body, artifact, MCP frame, or
  game-mod contract changes. Native Mod production, MCP registration, Harness use, deployment, and
  release remain `unverified`.

- Admit the refused-launch-contract recovery code on the legal-action read. The game-mod answers a
  refused launch contract with `503 launch_contract_refused`, or the prefix, `_`, and one bounded
  reason token (`sts2-game-mod#185`, `#187`), while this validator admitted only
  `host_not_configured` and `host_observation_unavailable`, so a refusal was relayed as a generic
  failure and the mod's vocabulary never reached a consumer on this route. The admitted set is now
  the producer's own rule rather than a second list: the bare prefix, or the prefix, `_`, and a
  token of 1 to 64 ASCII alphanumerics, `_`, or `-`. A code the mod cannot compose — a trailing
  separator, a dot, a slash, a space, a non-ASCII byte, a 65-byte token, or a neighbouring string
  that merely starts the same way — still fails closed, as do every other status, route, key set,
  correlation mismatch, duplicate key, and oversized body. See
  [ADR 0014](docs/decisions/0014-runtime-v3-framing-and-fencing.md), refs #85. The two consumers
  that mirror the same three-code set — `sts2-mcp-server` `catalog_reobserve.rs`, and `sts2-harness`
  `runtime_v3_wire.rs` with its ADR 0010 — are deliberately unchanged here and remain `unverified`
  on this route.

- Wire the approved-profile process lifecycle into the attached runtime through three fixed,
  lease-fenced, authorization-scoped routes: `POST /v1/instances/{instance}/process-lifecycle/operations`
  (`Mutate`), `GET /v1/instances/{instance}/process-lifecycle` (`Read`), and
  `GET /v1/instances/{instance}/process-lifecycle/operations/{id}` (`Read`), contract
  `sts2-gateway-process-lifecycle-v1`. Before this, `service_routes.rs` had no dispatch for the
  lifecycle component at all, so an attached deployment could compose a coordinator and still have
  no route that authenticated, fenced, and dispatched an operation — the named contract gate for
  `sts2-gateway#50` items 2 and 4 and for `sts2-harness#101`. A submission body carries only an
  opaque operation id, an authority epoch, and one closed action: instance, caller, session, lease,
  and lease epoch come from gateway configuration, and executable/install/image/user-data/process
  policy are never expressible on the wire, because the server-owned catalog resolves an opaque
  profile id. Configured string identities are bridged to the coordinator's numeric identity space
  by a deterministic, domain-separated SHA-256 truncation masked to 63 bits, because the durable
  store keys records by `i64`. Lease liveness stays the HTTP gate's decision while the lifecycle
  fence port decides identity only, so the two cannot disagree about expiry. The shipped binary
  validates the configured catalog, capacity budget, and durable store at startup but composes no
  concrete OS process adapter (ADR 0024 defers it), so a configured deployment advertises
  `available: false` and refuses every effect with `503 process_lifecycle_adapter_absent`, while an
  unconfigured deployment stays byte-identical and falls through to `404 route_not_found`. The
  change is additive: no existing route, body, protocol artifact, MCP frame, or game-mod contract
  changes, and `ProcessLifecycle::bind_attached_lease` is a new public method while `Lease::new`
  remains `pub(crate)`. Four real defects were found and fixed by the focused tests: an inverted
  per-action field guard that rejected valid launches, profile entries that bypassed the component
  validators, a bound lease whose expiry was derived from the request rather than stated explicitly
  (unsafe under any expiry-checking fence port, including the crate's own default), and an identity
  digest that overflowed the store's `i64` key. The first, second, and fourth are pinned by mutation
  probes; the third is pinned from the HTTP gate's side, since the attached fence port deliberately
  never reads the expiry field. See
  [ADR 0035](docs/decisions/0035-attached-process-lifecycle-route-surface.md). Native OS process
  launch/stop evidence (`#50` AC5) and the harness-side client mapping (`#101`) remain `unverified`.

- Stop a parallel-load flake in the runtime-v3 recovery translation test helper: `run_runtime_v3_translation_case_with_host_status` created one 3-second `accept` deadline before its wait loop and shared it across all three waits, and required a `/api/v3/runtime/state` probe even in the case that deliberately expires the recovery lease mid-query, where the gateway may legitimately never issue one. Each wait now carries its own budget, deadline exhaustion reports a named timeout instead of a bare socket error, and the probe is optional exactly when the lease is expected to have expired. The case still fails when the deadline enforcement is removed.

- Fix the live-observation bootstrap request-limit check: `request_limits_valid` compared the
  request's declared `max_item_bytes` and `max_message_bytes` against `MAX_RESPONSE_BYTES`, the
  131072-byte transport framing ceiling, instead of the maxima the pinned bootstrap schema declares
  (65536 and 262144). The pinned golden request declares the schema's 262144 message ceiling, so it
  was rejected as invalid before any producer call and the harness observed the bootstrap as
  unavailable. The ceilings now live in named constants next to the profile, and two tests bind them
  to the schema and to the untouched golden request so the pair cannot drift again; the bootstrap
  tests previously rewrote the golden's `max_message_bytes` down to 131072, which hid the drift, and
  no longer do. This is source/component evidence; native host behavior and integrated readiness
  remain unverified.

- Add a real filesystem isolated-allocation adapter for save-profile provisioning
  (`FilesystemUserDataPort`). The adapter binds one canonical, server-configured root and allocates
  exactly one `run-<identity>` directory per opaque identity, recording gateway provenance inside
  that directory. Traversal, symlink escape, unknown or foreign existing contents, capacity
  overrun, and implicit overwrite or adoption are all refused before any write, and host paths never
  appear in a descriptor or an error. This supplies the physical allocation and containment half of
  issue #51 at the component level; the attached runtime still injects no adapters unless a durable
  store path and user-data root are configured, so mutations keep failing closed by default. See
  [ADR 0024](docs/decisions/0024-save-profile-provisioning-and-fencing.md), refs #51.

- Add an explicitly negotiated repeated-episode lease profile
  (`x-sts2-episode-profile: repeated-episode-lease-v1`) so two completed
  episodes can run against one deployment without weakening the single-episode
  default. A profiled release of a live episode, after a confirmed host revoke,
  records the completed epoch as an admission floor and reopens admission for a
  strictly higher epoch of the same boot; a release without the header stays
  byte-identical and permanently revoked, and operator revoke, shutdown,
  unresolved host revoke, stale fence, rotated boot, and wrong caller/session
  all keep admission closed. The profile is gateway-local process state and is
  not written to durable boot or release-set authority. This is Gateway
  component evidence from the durable store and signed host frames over TCP
  loopback; native execution and the downstream watchdog soak remain separate.
  See
  [ADR 0033](docs/decisions/0033-repeated-episode-lease-profile.md), refs #67.

- Fix stop precedence over the repeated-episode lease profile. A release that
  arrives while a stop is already in force (an operator revoke whose host
  acknowledgment was lost, a shutdown, or an allocation-cleanup retry) completes
  the pending revoke but no longer clears the permanent stop flag or arms the
  profile, and a header-less release no longer echoes an earlier episode's
  stored witness. The reopen decision now uses the admission state captured
  before the release revoked the lease, because the revoke sets the permanent
  flag unconditionally. Regression coverage lives in
  `service_episode_stop_precedence_tests.rs`. See
  [ADR 0033](docs/decisions/0033-repeated-episode-lease-profile.md), refs #78.

- Add the authenticated, fixed `game-information-live-observation-bootstrap-v1`
  route. The Gateway pins schema digest
  `6041a282ffda8757af4e3eb6ab551e082f136fe53138ab8ac17db9fab52765c2`, requires a
  current lookup-binding scope and lease fence, validates parent and per-entity
  snapshot identity and response bounds, and preserves an explicit native
  `not_observable` refusal. The negotiated offer is gated by the explicit
  `STS2_GAME_INFORMATION_LIVE_BOOTSTRAP_ENABLED` installed-handler setting and
  current binding witness; native Mod production remains separate. See
  [ADR 0032](docs/decisions/0032-game-information-live-observation-bootstrap-route.md).

- Add the authenticated `exact_restore` Gateway transport for the frozen
  exact-restore neutral protocol and MCP wrapper. Five fixed routes validate the
  configured principal, capability, complete current owner fence, request and
  response schemas, correlations, and 16 KiB frame bounds on every phase before
  forwarding only to the matching fixed native path. A native
  `REJECTED/no_restore_adapter` reply stays explicit and does not permit uploads.
  This confirms Gateway source/component transport only; native restore and
  Harness continuation remain separate work. See
  [ADR 0031](docs/decisions/0031-exact-restore-gateway-transport.md).

- Add the separate `sts2-continuation-owner-adopt-v1` route for resuming an
  already claimed, still-live destination. It returns the original claim and
  the durable allocation recovery authority without allocating, renewing, or
  mutating the host. The published continuation-owner-v1 schema and digest
  remain unchanged. Gateway component evidence covers live-owner and claim
  fencing; Harness resume wiring and native restore remain separate work. See
  [ADR 0029](docs/decisions/0029-continuation-owner-adoption.md).

- Add the versioned continuation-owner read, claim, and historical lookup
  routes. Durable claims bind one logical continuation operation to an exact
  current gateway lease/fence and reject sibling or stale claims. Historical
  claims never establish live ownership; restart without a confirmed in-memory
  lease remains `unknown`. The contract is pinned in
  `contract-artifact/continuation-owner-v1`. Synthetic SQLite and production
  dispatcher evidence is covered; native restore and continuation remain
  unavailable pending their owning contracts and effects. See
  [ADR 0028](docs/decisions/0028-continuation-owner-fence.md).

- Add the authenticated lookup-binding route for the pinned
  `game-information-lookup-binding-v1` profile. The gateway forwards only the fixed producer path
  after current lease admission, validates raw JSON against the copied schema, binds scope,
  harness authority epoch, instance, locale, correlation, and canonical binding ID, and rejects
  malformed, duplicate, foreign, oversized, or status-inconsistent responses before success.
  Observation now requires a current matching discovery scope, authority epoch, binding, and
  manifest before producer I/O; missing or stale discovery returns
  `409 game_information_lookup_binding_discovery_required` without forwarding. A mismatched
  validated producer observation returns 502 and clears the retained discovery, so callers must
  re-discover after authority changes.
  Synthetic route evidence is confirmed; producer and live host behavior remain unverified. See
  [ADR 0025](docs/decisions/0025-game-information-lookup-binding-route.md).

- Add a bounded SQLite store for forwarded save-profile operation intents and results (#51).
  Private versioned records, atomic coordinator fencing and exact request/result validation
  support reopen-and-lookup recovery without another create/select dispatch. This store is
  separate from allocation persistence; attached runtime mutations remain disabled until
  complete owner adapters and authoritative active-run admission are supplied.

- Harden the merged save-profile runtime composition (#51). The attached runtime no longer
  constructs in-memory provisioning or operation-intent substitutes: it injects no
  isolated-allocation port, launch-profile binding port, durable intent store, or authoritative
  active-run source, so every mutation fails closed before provisioning or forwarding with
  `save_profile_active_run_unavailable`, `save_profile_persistence_unavailable`, or
  `save_profile_provisioning_unavailable` while discovery reads keep forwarding through the fixed
  loopback targets. Launch bindings are produced only by an injected `LaunchProfileBindingPort`,
  creation receipts must echo the reserved allocation identity and the approved launch contract,
  and a refused launch binding or refused allocation blocks the operation. Regression tests cover
  the unprovisioned capability results, injected binding forwarding, receipt-identity binding,
  distinct-operation duplicate selection, typed allocation-port refusals, and reopened-store
  restart replay without a second dispatch. Real filesystem isolation, a production durable store,
  issue #50 launch-profile wiring, and cross-restart durability remain unverified.

- Add the gateway-owned profile lifecycle contract for issue #50. An opaque approved
  `LaunchProfileId` resolves to bounded executable/install/image identity, isolated user-data
  namespace, and process policy; authenticated launch, identity-checked attach, stop, restart,
  durable intent/replay, and explicit `Blocked`/`Unknown` cleanup outcomes are covered by
  deterministic process-port fixtures and an SQLite record seam. Durable per-instance ownership
  reservations, server-issued operation ordering, repeated read-only recovery, and an explicit
  no-eviction operation-record budget now prevent duplicate launch, stale-history replacement,
  and unbounded retention. Ambiguous launch faults stay `Unknown` with a durable reservation and
  retain any identity-bearing cleanup obligation until exact, child-free absence is proven;
  approved user-data namespaces cannot be reused concurrently, and on-disk SQLite coordinators
  are fenced by an exclusive lock plus a durable coordinator token and transactional admission.
  This is source/component evidence only: native process launch, host
  readiness, harness workflow mapping, and deployment compatibility remain unverified behind the
  `sts2-harness` prerequisites. Legacy ports reject the profile-aware launch path before
  starting; consumers with exhaustive matches over the expanded public lifecycle/fault enums must
  add arms.
- Add the authenticated, bounded `game-information-query-v1` read transport for capabilities and
  the canonical envelope query route (with additive operation-specific aliases). Fixed
  instance-scoped routes derive only their allowlisted loopback producer paths after
  caller/session/lease/epoch and static content/live snapshot fencing. A successful capabilities
  response is required for the current producer authority; advertised operations and negotiated
  limits are enforced before forwarding, caller disconnect cancels owned producer work, request,
  response, page, item, queue, and timeout budgets are enforced, typed producer errors are
  preserved, and no response cache or cross-instance fallback exists. The gateway pins protocol
  source commit
  `34f68b182c09472c3a0573ff478e17e6ed53c91f` at schema digest
  `376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9`. Synthetic gateway
  transport evidence is confirmed; native producer, MCP (#51/#52), harness, deployment, and
  release compatibility remain unverified. See [ADR 0024](docs/decisions/0024-game-information-query-routing.md).
- Add the proposed gateway-local save-profile component from
  [ADR 0024](docs/decisions/0024-save-profile-provisioning-and-fencing.md): fixed fenced
  list/current/select/create-disposable/lookup routes, fresh opaque user-data allocation with
  provenance and traversal/symlink/overwrite refusal, retained operation intent, and explicit
  timeout/disconnect/unknown reconciliation. Deterministic source/component tests pass; game-mod
  contract acceptance, issue #50 launch-profile wiring, production persistence, native save
  behavior, and cross-restart durability remain unverified.

- Add an authorized, bounded public checkpoint-reference read route with closed response
  validation and explicit unavailable results when no producer exists. Native capture remains
  unverified.

- Bind each enabled `coop-native-v1` local producer route to one configured peer token, peer ID,
  and instance/session/lease/epoch tuple. The gateway rejects caller-selected peer substitution,
  stale bindings, duplicate pending operations, unbound recovery, mismatched returns, and changed
  returned authority rather than correlating by a fingerprint, actor, or generation. Every valid
  returned observation must also identify that canonical peer as its sole `local` peer before the
  response is returned or pending state can be cleared. This is a
  gateway component safety correction; `coop-native-v1` bytes/digest are unchanged and native
  carrier, two-peer, host-settlement, and live-runtime behavior remain unverified.

- Forward the recovered instance, lease ID, and epoch to the fixed loopback mod boundary after
  recovery admission, rather than stale process configuration values. This closes a gateway-side
  fencing mismatch; native co-op restart/rejoin settlement remains unverified.

- Reject native co-op accepted/unknown action, vote, and pending-rejoin receipts when their
  before-generation differs from the request fence. A stale rejected response may still report
  the newer host generation without claiming mutation admission.

- Reject a status-null `recovery_response` echoed by the downstream as a response; that
  bodyful shape is admitted only as the recover request and cannot be reported as success.

- Fence unknown recovery receipts to the observed host generation: only an accepted pending
  rejoin may carry an explicit same-generation `after_host_generation`; reconcile and unresolved
  receipts retain a null after-generation until recovery settles.

- Add the accepted `coop-native-v1` gateway consumer. The six fixed instance-scoped routes
  validate the copied closed schema and authenticated identity/lease headers before forwarding
  only the matching game-mod observation, legal-catalog, action, vote, rejoin, or recovery path.
  Settled effects, receipts, generations, peer observations, and recovery lineage are checked at
  the gateway boundary; bounded producer admission errors remain explicit. This is
  source/component evidence at schema digest
  `2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629`; native host settlement,
  two-peer gameplay, provider execution, deployment, and release compatibility remain unverified.

- Add the authenticated, fixed-route host lease-control consumer for gateway-issued
  install, renew, and revoke acknowledgments. Protected grant persistence,
  fail-closed admission, and operation identity reconciliation are included;
  managed-host settlement and live deployment remain unverified.

- Keep a persisted `INSTALLED` host-install row historical across a gateway
  process restart. Mutation admission and idempotent install replay now require
  the exact current-process grant cache; a missing or mismatched cache fails
  closed until a fresh host acknowledgment is obtained.
- Add the candidate `runtime-v4-expert-rest-action-v1` dispatch and reconciliation routes at
  `POST /v4/instances/{instance_id}/expert-rest-action` and
  `GET /v4/instances/{instance_id}/expert-rest-actions/{operation_id}`. The gateway pins the
  `expert-rest-action` artifact at schema digest
  `bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd`, enforces authenticated
  lease and correlation fences, bounds fixed JSON forwarding, retains selector admission context,
  and validates settled observation, transition, catalog, and effect-witness relationships. The
  copied artifact remains a candidate with no admitted consumers; native producer, host settlement,
  MCP/harness integration, deployment, and release evidence remain unverified.

- Add the proposed, read-only `coop-receipt-query-v1` route at
  `POST /v1/instances/{instance_id}/coop/receipt-query`. The gateway validates the exact
  schema digest, provenance, canonical UTF-8 envelope, repeated identity, retained-receipt
  semantics, and active lease before forwarding only `/api/v1/coop/native/receipt-query`.
  The copied profile remains `proposed_unadmitted` with no admitted consumers; native host
  compatibility, live receipt production, and cross-consumer replay remain unverified.

- Tighten Runtime-v4 expert settlement fencing at source head `aecc9fa44c825623b3e3bbb21d130e1fe6ac9468`: bind nested observation `state_id` and `generation` to the outer response and dispatch transition `before_generation` to the request generation. Independent source/component checks pass; native host legality, settled effects, provider execution, cross-consumer integration, deployment, and release remain unverified.

- Add the bounded `seeded-run-v1` gateway seam at current gateway main
  `2b44bf347f790509c9f13378c89719d09366d45b`, consuming protocol main
  `d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404` at schema digest
  `5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8`: fixed instance-scoped public routes, fixed native
  start and receipt routes, complete selected-context and settlement validation, semantic
  idempotency with correlation rebinding, accepted/unknown read-only reconciliation, and an
  opt-in journal sidecar for restart recovery. Component evidence covers the ledger and journal;
  native host settlement, real installation compatibility, save isolation, provider execution,
  gameplay, and release remain unverified.

- Tighten Runtime-v4 expert settlement fencing at historical source head `aecc9fa44c825623b3e3bbb21d130e1fe6ac9468`: bind nested observation `state_id` and `generation` to the request generation. Independent source/component checks pass; native host legality, settled effects, provider execution, cross-consumer integration, deployment, and release remain unverified.

- Add the gateway-local Runtime-v2 workflow authority and recovery contract: owner boot/fence
  identity, independent recovery-domain capabilities, and recovery-only retained receipt access.
  Stale or implicit workflow admission, missing/changed workflow boot identity, and unavailable
  receipt retention fail closed. The attached Runtime-v2 action, state and reconcile routes now
  install the contract and require the matching opt-in `x-sts2-workflow-boot-epoch` header when
  `STS2_WORKFLOW_BOOT_EPOCH` is configured; the frozen Runtime-v2 envelope remains unchanged.
  Evidence is limited to deterministic gateway source/component tests.

- Add the bounded Runtime-v4 expert-state, expert-action, and expert-reconcile gateway routes.
  The source/component implementation validates the checked-in observation and action artifacts
  at `17b93bf`; native host legality, settled effects, provider runs, and end-to-end compatibility
  remain unverified.

- Add the additive `runtime-map-v1` read-only snapshot route. The gateway forwards only
  `GET /api/map/v1/snapshot` after its existing lease and identity fences, validates the corrected
  protocol artifact at merged main commit `b3d3034f32e68d70c9e681f906ee37d74db153c4` and schema digest
  `ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b`, and bounds responses at
  256 KiB. Graph and binding validation preserves overlapping coordinates and disconnected visible
  components while rejecting stale, foreign, cyclic, duplicate, or malformed data. Payload text is
  bounded in UTF-8 bytes, excludes C0/DEL/C1 controls, and enforces elapsed timeout ordering. Live
  host map observation and visualizer compatibility remain unverified.

- Record the merged current gateway main source head `77782d5745a8c1f3399807d48138c5c7b511bff1` for
  the bounded `runtime-map-v1` route and copied-artifact consumer. This is source/component and
  artifact-copy evidence; native map visibility, navigation, gameplay, release, and publication
  remain unverified.

- Add opt-in coordinator-reported co-op synchronization: configured roster, control-scoped
  fenced reports, monotonic convergence and expiry, and a read-only response consumed by
  the executable MCP profile. Both routes avoid downstream game access. The copied protocol
  artifact, deterministic ledger tests and real gateway/MCP transport gate are included. The
  executable check is coordinator-report evidence only; it does not establish native peer
  identity, shared game effects, or multiplayer gameplay.

- Record bounded native runtime-v3 Windows/Linux campaign and replay paths through an attached
  mod listener. The dated records cover the named v0.107.1 fixtures and Defeat outcomes; process
  lifecycle, native multiplayer, and broader compatibility remain unverified.

- Consume the coordinated Runtime-v3 continuation schema with argument-free proceed,
  confirm-selection and cancel-selection actions; reject mixed revisions and extra arguments.

### Fixed

- Validate recovery allocation authority against the current lease, durable fence,
  installed host binding and monotonic deadline. Failed responses close local
  admission before fallible revocation; busy storage cannot reopen mutation.
  Confirmed host cleanup permits a fresh epoch without clearing prior stop intent.
  Bind delayed cleanup permission to the exact failed allocation; reject installed
  grant digest mismatch and retire a lease when activation expires after its
  durable host installation acknowledgment.

- Default MCP transport identity independently to `mcp-session-1` to match MCP and harness
  configuration; retain validated explicit overrides and the complete session fence.

- Integrate Exo routes with Runtime-v2 component queue, journal, scoped authentication and MCP
  session fences; require mutate scope for dispatch and control scope for recovery.

- Preserve narrowly validated legal-catalog refusal errors (stale generation/unavailable host)
  as HTTP409/503 with an explicit reobserve hint; never treat them as a successful catalog.

- Split the independent Runtime-v2 component wiring from PR #6 at
  `3cf7f08f36daf31ca2d9cc3e455a622db78d68af`; retain its original branch and commits for review.
  The separate Exo gameplay lane owns Runtime-v3 integration.
- Complete the inert Runtime-v1 protocol copy and check frozen v1/v2 inventories in CI; preserve
  existing schema/manifest bytes and distinguish attached adapters in the repository layout.
- Reject omitted required nullable Runtime-v2 envelope members during decoding while preserving
  explicit null values and the frozen artifact bytes.

- Bound attached HTTP request/reply and downstream exchange lifetimes with absolute five-second
  deadlines, and reject oversized or ambiguous header framing. Require literal loopback endpoints
  and prevent reallocation of a released attached lease context during the same process lifetime.
- Replay exact authenticated Runtime-v2 operation receipts before fresh-action generation checks;
  reconcile Accepted as well as Unknown work and prevent late receipts from rewinding observation.
- Include executable Rust sources under `src/bin` in repository policy; split attached service
  concerns under unchanged file budgets and regression-test the actual scanner's coverage.

- Keep a released/shutdown attached lease revoked for the service lifetime; a later allocation
  cannot resurrect the same credential/epoch and authorize queued stale work.

- Reject attached action operation IDs that cannot be represented by the fixed receipt route,
  before dispatch; keep neutral ledger identity and frozen artifact bytes unchanged.

- Remove the policy scanner's blanket `bin` exclusion, split attached runtime source/tests by
  responsibility, and verify the runtime entrypoint and routing/HTTP source remain scanned under
  unchanged size limits without exemptions.

- Enforce numeric loopback endpoints, absolute bounded HTTP I/O, unambiguous HTTP framing, and
  connection-owned shutdown cancellation; reserve queue metrics before publishing work.

- Preserve the newest Runtime-v2 observation when reconciling an older operation receipt; reject
  regressed state refresh and inconsistent persisted result generations. Accepted and unknown
  operations retain historical settled receipts without rewinding fresh-action admission.
- Bound journal reads before allocation, create exclusive private temporary files without following
  existing temporary-path links, and sync the current directory for relative journal paths on Unix.
- Failed generic process starts no longer consume an unreachable allocation slot; consumed instance
  and lease identities remain unique. Clarified the process port's partial-start cleanup ownership.
- Expiry reconciliation reports forced-stop failures instead of claiming successful expiration,
  preserving the process handle and revoked lease for explicit cleanup retry.
- Correct co-op snapshot authorization without a local peer and permit synchronization to recover
  after every connected peer reaches a common newer generation, without lowering the baseline.
- Retain exact Runtime-v2 operation replays across generation changes, reconcile accepted work,
  and preserve newer observations when historical completion receipts arrive late.

- Bound incoming HTTP reads and outgoing writes by absolute five-second deadlines; slow-drip
  clients cannot extend the deadline. The downstream connect/write/read exchange shares one
  five-second budget. Reject oversized terminated headers and ambiguous transfer framing.
- Require literal loopback addresses and nonzero ports for both listeners and downstreams;
  released attached lease contexts cannot be allocated again during the same process lifetime.
- Validate Runtime-v3 requests and responses against the copied canonical gameplay schema,
  matching route kinds, authenticated envelope identities, correlations, operations, metadata,
  and neutral semantic relationships. Duplicate JSON fields and undeclared fields are rejected.
  These corrections do not implement durable restart epochs, lease TTL/renewal, or a real host.

### Added

- Add the fixed, authenticated `POST /v1/recovery/host-fence` bridge for the
  additive recovery sideband. The bounded downstream request carries the new
  boot/fence identity in its closed frame and does not require or forward an
  old gameplay lease; transport uncertainty remains explicit.

- Record the owner-accepted `jsonschema` product dependency and its conditions in ADR 0015;
  add a self-check test that the embedded Runtime-v3 schema compiles and admits a golden request.

- The bounded Runtime-v3 gameplay route allowlist and forwarder, gateway-owned co-op peer
  synchronization, and an injected process supervisor. For this source and component test
  entry, live launch, host settlement, and multiplayer traces were unverified; later dated
  campaign records are scoped separately.
- A bounded injected-process restart seam that removes the old owned handle before replacement
  start and fails closed when replacement start fails.
- The frozen Runtime-v2 gateway operation ledger and fixed forwarding seam: full envelope and lease
  validation, bounded operation keys, canonical duplicate/conflict checks, exactly-once dispatch,
  retained-receipt reconciliation, explicit unknown/cancelled outcomes, capacity fencing, and the
  conceptual `/v2/instances/{instance_id}/action` and `/operations/{operation_id}` routes.
- A copied Runtime-v2 release-like artifact from protocol handoff commit `8d4b2f5`, including the
  exact schema digest `f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2` and checksum
  verification. The deterministic fake seam is confirmed; live gameplay settlement is unverified.
- Repaired the fixed Runtime-v2 state route to emit a typed request with explicit unavailable status
  when no host adapter is configured, fenced duplicate/receipt reads by current identity and
  generation, and made the in-process artifact verifier calculate every listed SHA-256 with tamper
  coverage.
- Added an optional bounded Runtime-v2 journal with atomic replacement, admission/terminal
  checkpoints, restart-to-unknown recovery, settled-receipt replay without downstream mutation, and
  fail-closed identity validation. The journal now holds an exclusive process-lifetime lock per
  configured path and syncs the parent directory after replacement on Unix. Exact duplicate replay
  now precedes generation revalidation, and the attached bearer check uses a length-independent byte
  comparison.
- Added the bounded `STS2_RUNTIME_V2_OPERATION_CAPACITY` setting (1 through 64) and deterministic
  overload/persistence/authentication tests. This remains a single-instance component lane; it does
  not claim global backpressure, process supervision, four-instance isolation, or live host support.
- Added a single-worker FIFO admission queue configured by
  `STS2_RUNTIME_V2_QUEUE_CAPACITY`, typed 429 overflow, sanitized authenticated metrics, and a
  lease-fenced shutdown route that explicitly cancels queued requests. This is component-level
  backpressure and lifecycle evidence, not a production multi-instance supervisor.
- Added gateway-local credential scopes, current/previous token rotation overlap, bounded expiry
  checks, and stable 401/403 failures before queue admission. Credential issuance, revocation, and
  downstream secret rotation remain external responsibilities.
- Added the configured `STS2_MCP_SESSION_ID` lease fence. The attached runtime now rejects a missing
  or mismatched `x-mcp-session-id` before forwarding, while retaining the frozen Runtime-v2 envelope
  and defaulting to the gateway session for compatibility.
- Added deterministic four-instance control-plane coverage for independent caller/session fences,
  capacity exhaustion, survivor readiness, release, and terminal cleanup. This remains fake
  control-plane evidence and does not claim process-supervisor or host isolation.

- The bounded `sts2-gateway-runtime` attached single-instance loopback adapter with bearer
  authentication, allocation/release, lease fencing, fixed runtime routes, and `runtime-v1`
  artifact reference.

- Confirmed the attached adapter in the authorized exact-host coordinator trace through the managed
  game-mod runtime probe.

- A verbatim offline `sts2-protocol/poc-v1` artifact copy from the normative protocol source, with
  checksum validation, complete manifest provenance/path checks, and a deterministic POC request
  oracle covering fake allocation/readiness, fixed-route forwarding, stale lease fencing, and
  wrong-instance rejection before transport.
- Repository governance, target-local policy checks, CI workflows, and gateway boundary documents.
- Decisions for gateway ownership/dependencies and the current sixth-target protocol boundary.
- A target-owned `sts2-gateway` Rust package with in-memory lifecycle control, explicit process,
  readiness, transport, and lease-decision ports, plus deterministic fake-instance tests.

### Not implemented

- Generic process adapters, game rules, host integration, and live host runtime behavior remain
  outside this attached adapter. The component binary is intentionally fixed to one attached
  downstream instance; production storage durability, broader lifecycle, and host behavior remain
  runtime-unverified.
