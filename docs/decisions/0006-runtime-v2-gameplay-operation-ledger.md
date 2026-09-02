# ADR 0006: Runtime-v2 bounded gameplay-operation ledger

- Status: Accepted for the deterministic gateway seam; live gameplay settlement remains unverified
- Date: 2026-09-02

## Context

Runtime-v2 introduces one deliberately narrow gameplay mutation, `end_turn`, with a protocol-owned
envelope and explicit admission, settlement, rejection, cancellation, and uncertainty outcomes. A
gateway acknowledgment cannot prove that a downstream mutation settled, and a caller timeout or
disconnect cannot safely determine whether a write reached the downstream runtime.

The gateway must retain enough identity to replay one request without applying it twice, distinguish
conflicting reuse, fence stale instance/session/lease/epoch/generation context, and reconcile an
uncertain write without issuing a second mutation. The implementation must remain independent of the
protocol repository's Rust crate and must not guess a host API.

## Decision

The gateway owns a bounded in-memory Runtime-v2 operation ledger and a dedicated
`RuntimeV2ForwardingPort`. The operation key is the tuple
`instance_id + session_id + lease_id + lease_epoch + operation_id`. Each new action stores the
canonical serialized request identity and its result before returning it. An exact duplicate
replays the retained action response; a different request under the same key returns a rejected
`idempotency_conflict` response without dispatch.

The ledger validates the copied Runtime-v2 metadata, provenance, action identity, bounded
observation, current identity fence, and exact current generation before dispatch. Semantic
rejection is retained and cannot mutate. A post-write timeout or disconnect is retained as
`unknown` with no observation or witness. Reconciliation calls only the forwarding port's
read-only retained-receipt method; it never calls the mutation method. A `settled` result is accepted
only with a fresh post-action observation and a matching `turn_end_settled` witness. Cancellation
exists only before dispatch and cannot undo admitted work.

The attached runtime binary owns the fixed routes
`POST /v2/instances/{instance_id}/action` and
`GET /v2/instances/{instance_id}/operations/{operation_id}`. It validates the gateway bearer
token, exact lease headers, correlation header, body bounds, and full message envelope. Its current
v2 downstream seam is deliberately unconfigured and fails closed before write; no host path or
gameplay API is inferred from Runtime-v1.

## Compatibility and retention

Runtime-v1 routes and types remain unchanged. Runtime-v2 consumes the copied release-like artifact
with schema digest
`f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2`, kept in the replaceable
`runtime_v2_artifact` module. The ledger retains entries until process restart or its fixed store
capacity is reached; it does not evict or persist entries. Restart retention and live downstream
compatibility are unverified. A caller must not blindly retry an unknown operation after restart.

## Deterministic oracle

The fake forwarding seam must show exactly one dispatch/application for an exact duplicate, one
read-only receipt lookup for unknown-to-settled reconciliation, no dispatch on duplicate conflict,
stale epoch, cancellation, or capacity rejection, and no dispatch retry when reconciliation returns
no receipt. These checks are source/test evidence only and do not authorize live STS2 gameplay.
