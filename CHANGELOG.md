# Changelog

All notable changes to `sts2-gateway` are recorded here. This target does not publish a concrete
runtime or downstream wire contract.

## [Unreleased]

### Fixed

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

- Generic process adapters, persistence, game rules, host integration, and live host runtime behavior
  remain outside this attached adapter. The component binary is intentionally fixed to one attached
  downstream instance; broader lifecycle and host behavior remain runtime-unverified.
