# `game-information-query-v1`

This artifact defines a bounded, read-only query/result envelope for static content and live
instance details. It is owned by `sts2-protocol`; the named consumers are `sts2-game-mod`,
`sts2-gateway`, `sts2-mcp-server`, and `sts2-harness`. The artifact carries no route, host,
authentication, lifecycle, persistence, or mutation behavior.

The gateway consumer pins protocol source commit
`34f68b182c09472c3a0573ff478e17e6ed53c91f` and schema digest
`376845b0c86b4afcd2c79ffba753eb7e7e416f5410da26b4dae970cfee2221d9`. This is a copied,
release-like artifact pin; it does not claim a live producer or host-compatible deployment.

Definitions are identified by
`(content_manifest_id, entity_kind, namespaced_id, variant)`. Live occurrences use the distinct
`(instance_id, run_id, epoch, entity_kind, entity_id)` tuple. A localized display name is never an
identity. Static bindings name a content manifest, locale, and visibility scope. Live bindings add
an instance fence and an immutable `snapshot_ref` with its state generation.

Every response reports deterministic ordering, bounded item/page/text accounting, final-page and
known-total semantics, coverage, and per-field availability. Byte accounting is reproducible:
canonical compact JSON is encoded as UTF-8; `item_bytes` is the largest encoded item object,
`payload_bytes` is the encoded `items` array, `page_bytes` is the encoded page with its
`accounting` member omitted, `text_bytes` is the sum of available `text` and `text_list` values,
and `max_message_bytes` bounds the complete envelope. Available zero and empty values are not
unavailable values; unavailable, redacted, unsupported, missing, and not-observable fields carry
`null` and a reason. Each field names its producer source. An opaque cursor is bound to the
normalized query, content revision, locale, scope, limits, and (when live) instance/snapshot
fences. Reuse across any binding change is rejected as `stale_cursor`; a stale retained snapshot
is rejected as `stale_snapshot`.

The snapshot policy declares a generation lifetime, retained-snapshot bound, and invalidation on
restore, restart, profile/content/run changes, or epoch changes. `query_request`, `query_response`,
`capabilities_response`, and `error_response` are explicit negotiated shapes. Unknown versions,
enum values, IDs, fields, filters, scopes, and oversized values fail explicitly; the profile has no
write operation.

The goldens and field vectors are synthetic UTF-8 data. Rust and JavaScript witnesses validate the
same bytes and vector IDs. This is protocol/serialization evidence only; consumer source adoption,
gateway/MCP tool registration, host extraction, live snapshots, and end-to-end agent reachability
remain unverified until the named owners pin this digest.
