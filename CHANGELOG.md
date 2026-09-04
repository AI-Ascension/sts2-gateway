# Changelog

All notable changes to `sts2-gateway` are recorded here. This target does not publish a concrete
runtime or downstream wire contract.

## [Unreleased]

### Fixed

- Failed generic process starts no longer consume an unreachable allocation slot; consumed instance
  and lease identities remain unique. Clarified the process port's partial-start cleanup ownership.
- Expiry reconciliation reports forced-stop failures instead of claiming successful expiration,
  preserving the process handle and revoked lease for explicit cleanup retry.
- Preserve the newest Runtime-v2 observation when reconciling an older operation receipt; reject
  regressed state refresh and inconsistent persisted result generations. Accepted and unknown
  operations retain historical settled receipts without rewinding fresh-action admission.
- Bound journal reads before allocation, create exclusive private temporary files without following
  existing temporary-path links, and sync the current directory for relative journal paths on Unix.
- Failed generic process starts no longer consume an unreachable allocation slot; consumed instance
  and lease identities remain unique. Clarified the process port's partial-start cleanup ownership.
- Expiry reconciliation reports forced-stop failures instead of claiming successful expiration,
  preserving the process handle and revoked lease for explicit cleanup retry.

### Added

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
