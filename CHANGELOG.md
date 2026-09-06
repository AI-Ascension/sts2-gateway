# Changelog

All notable changes to `sts2-gateway` are recorded here. This target does not publish a concrete
runtime or downstream wire contract.

## [Unreleased]

- Consume the coordinated Runtime-v3 continuation schema with argument-free proceed,
  confirm-selection and cancel-selection actions; reject mixed revisions and extra arguments.

### Fixed

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

- Record the owner-accepted `jsonschema` product dependency and its conditions in ADR 0015;
  add a self-check test that the embedded Runtime-v3 schema compiles and admits a golden request.

- The bounded Runtime-v3 gameplay route allowlist and forwarder, gateway-owned co-op peer
  synchronization, and an injected process supervisor. Live launch, host settlement, and
  multiplayer traces remain unverified.
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
