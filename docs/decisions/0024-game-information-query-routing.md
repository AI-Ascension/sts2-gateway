# ADR 0024: bounded game-information query routing

Accepted for gateway source/component implementation; native producer and live host behavior
remain unverified.

## Requirement and ownership

Issue #52 requires authenticated gateway reads for the accepted
`game-information-query-v1` operations. The gateway owns route admission, authentication,
instance/session/lease/epoch fencing, bounded transport, and response validation. The game-mod
owns game observations and content meaning. The protocol artifact is copied as inert,
release-like data. MCP issues #51 and #52 are the named consumers; this change is the gateway
transport layer and does not add MCP tools or framing.

The gateway pins protocol source commit
`924acc650e5b6d57ecb9f602abe65caa3b025f53` and schema digest
`e5ba81b0520687cf59db6a94aea3b38606e86300f6eb2b0e858f55704e62f76c`. The copied artifact,
source-path schema, conformance case, synthetic fixtures, consumer pin, and checksum inventory
must remain byte-verified. A changed upstream artifact requires re-vendoring and a new pin.

## Fixed route and producer mapping

Only these operation-keyed mappings are admitted. The caller supplies an operation body, never a
downstream path or URL:

| Operation | Gateway method/path | Producer method/path | Request/response |
| --- | --- | --- | --- |
| capabilities | `GET /v1/instances/{instance_id}/game-information/capabilities` | `GET /api/v1/game-information/capabilities` | bodyless request; `capabilities_response` or typed `error_response` |
| list | `POST /v1/instances/{instance_id}/game-information/list` | `POST /api/v1/game-information/list` | `query_request` / `query_response` or typed `error_response` |
| search | `POST /v1/instances/{instance_id}/game-information/search` | `POST /api/v1/game-information/search` | `query_request` / `query_response` or typed `error_response` |
| get | `POST /v1/instances/{instance_id}/game-information/get` | `POST /api/v1/game-information/get` | `query_request` / `query_response` or typed `error_response` |
| detail | `POST /v1/instances/{instance_id}/game-information/detail` | `POST /api/v1/game-information/detail` | `query_request` / `query_response` or typed `error_response` |
| availability | `POST /v1/instances/{instance_id}/game-information/availability` | `POST /api/v1/game-information/availability` | `query_request` / `query_response` or typed `error_response` |

The route parser is closed over method, configured instance, and operation. Unknown operations,
methods, path segments, query parameters, redirects, arbitrary URLs, reflection forwarding, and
cross-instance or cross-profile fallback are rejected before a producer connection.

## Admission, scope, and readiness

Every route requires the existing read credential and exact caller, instance, gateway session,
MCP session, lease ID, lease epoch, and correlation headers. A stale, wrong, revoked, or
cross-instance lease fails closed before forwarding. Static queries must use public scope and
the configured content-manifest authority with null instance/snapshot fences. Live queries must
use player scope and match the configured run plus the selected instance, lease epoch, and
snapshot/parent state-generation fence. Definition references, instance filters, result items,
and cursor bindings retain the same content, locale, visibility, run, instance, epoch, and
snapshot identities. No response is silently retried against another snapshot or authority.

Capabilities are a read-only readiness check. The producer must advertise limits no larger than
the gateway budget and `snapshot_policy.supports_live`; an absent or malformed capability
response is unavailable/invalid rather than an implicit fallback. A producer typed
`error_response` keeps its status and body after correlation and schema validation.

## Budgets and cancellation

The request body is capped at 16 KiB and the response message at 256 KiB. Query limits are
bounded to 32 items, 65,536 page bytes, 4,096 text bytes, and 512 cursor bytes. The existing
single FIFO admission queue is shared with the attached runtime and is configurable from 1
through 64; it permits one active worker operation per service. Producer connect time is capped
at two seconds and the complete exchange at five seconds. HTTP framing rejects oversized or
incomplete bodies before parsing. A timeout, caller disconnect, or transport error drops the
owned read connection and returns a bounded typed outcome; it does not retry or leak the request
to another instance. There is no response cache. Cursor continuations are a bounded (64-entry)
in-memory binding registry, not a data cache, and compare the complete normalized query before
reuse; lease/epoch or content changes create a new service authority and cannot reuse it.

## Compatibility and deterministic oracle

This is additive. Existing Runtime-v1/v2/v3, Runtime-v4 expert, map, co-op, checkpoint, and
seeded-run routes and artifacts are unchanged. Rejection maps to stable gateway errors for
missing/invalid/oversized bodies, scope/limit failures, stale cursors, unavailable transport,
oversized or malformed responses, and preserved producer errors. The oracle invokes the
production dispatcher with synthetic loopback producers and asserts exact method/path/body and
identity headers for static list, live detail, and capabilities; unknown operations make zero
connections; stale/wrong lease and cross-scope requests make zero connections; oversized output,
timeout, cancellation, and continuation changes stay within the budgets. These tests establish
gateway source/component transport behavior only. Native extraction, host compatibility, MCP
tool registration, harness execution, deployment, and release compatibility remain unverified.
