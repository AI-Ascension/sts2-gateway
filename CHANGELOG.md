# Changelog

All notable changes to `sts2-gateway` are recorded here. Component evidence is separate from
host compatibility and release publication.

## [Unreleased]

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
