# Changelog

All notable changes to `sts2-gateway` are recorded here. This target does not publish a concrete
runtime or downstream wire contract.

## [Unreleased]

### Added

- A verbatim offline `sts2-protocol/poc-v1` artifact copy from the normative protocol source, with
  checksum validation, complete manifest provenance/path checks, and a deterministic POC request
  oracle covering fake allocation/readiness, fixed-route forwarding, stale lease fencing, and
  wrong-instance rejection before transport.
- Repository governance, target-local policy checks, CI workflows, and gateway boundary documents.
- Decisions for gateway ownership/dependencies and the current sixth-target protocol boundary.
- A target-owned `sts2-gateway` Rust package with in-memory lifecycle control, explicit process,
  readiness, transport, and lease-decision ports, plus deterministic fake-instance tests.

### Not implemented

- Concrete listeners, process adapters, authentication adapters, network transports, persistence,
  protocol dependencies, game rules, host integration, and live runtime behavior remain outside
  this initialization wave and runtime-unverified.
