# Changelog

All notable changes to `sts2-gateway` are recorded here. This target does not publish a concrete
runtime or downstream wire contract.

## [Unreleased]

### Added

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
