# `exact-restore-v1`

Status: accepted for protocol contract initialization; consumer adoption and native restore
remain pending. Canonical owner: `sts2-protocol`. The protocol schema and artifact are neutral
metadata only; they do not carry transport authentication, host authority, persistence code, or
game state. Named consumers are `sts2-game-mod`, `sts2-gateway`, `sts2-mcp-server`, and
`sts2-harness`; each boundary retains its own routing, authorization, storage, and effect owner.

The profile transfers a previously verified `ascension.checkpoint_manifest.v1` closure. It reuses
the existing `canonical_payload` and ordered `restore_artifacts` members and their role, digest,
size, and codec fields. It does not change `exact-state-v1`, add a checkpoint format, or assert
that a captured checkpoint is restorable. The Harness producer verifies the full manifest,
compatibility, coverage, every referenced blob, each digest and size, and the transfer limits
before it sends a request that can stage bytes.

## Frame and identities

Every frame is a closed RFC 8785 canonical JSON object encoded as UTF-8 without a BOM or trailing
newline. It carries `contract`, `schema_digest`, UUIDv4 `message_id` and `correlation_id`, a
closed `kind`, and a typed `payload`. Authentication and the outer transport envelope are owned by
the consuming boundary. Unsupported contract versions, schema digests, kinds, fields, or enum
values are rejected before an effect.

Each operation request carries a stable UUIDv4 `operation_id` already persisted with the selected
child's continuation claim and the complete non-secret `expected_owner` fence tuple. The owner
tuple names deployment, instance and incarnation, boot, authority generation, host fence and
generation, lease and epoch, session, and lease expiry. Begin binds that operation to the selected
experiment, branch, metadata revision, run, episode, trajectory, checkpoint, exact-state digest,
compatibility, coverage, and boundary. Later phase requests repeat the same operation and owner.
The protocol names these identities; the owning runtime boundaries determine their live validity.

Begin's `artifacts` array carries `canonical_payload` first, followed by all `restore_artifacts` in
manifest order. It preserves aliases and their declared role, digest, size, and codec. The
reference count includes aliases; the distinct blob count does not. Aliases with one digest must
agree on size and bytes. The Harness validates these conditions and computes the complete closure
before its first transfer request.

## Limits and closure integrity

The immutable profile limits are:

| Limit | Value |
| --- | ---: |
| Complete request frame | 16,384 bytes |
| Complete response frame | 16,384 bytes |
| Raw chunk | 8,192 bytes |
| Base64 chunk member | 10,924 bytes |
| Manifest artifact references, including aliases | 64 |
| Manifest or distinct blob | 16 MiB |
| Aggregate manifest and distinct blob bytes | 64 MiB |
| Unfinished operations per native instance | 1 |
| Retained terminal receipts per native instance | 256 |

The aggregate includes the manifest exactly once, plus each distinct blob exactly once. Blob
deduplication is scoped to `canonical_payload` and `restore_artifacts`; even if a blob's bytes
happen to equal the manifest bytes, both are counted. Arithmetic is overflow checked. The artifact
list's request-frame size can impose a stricter effective limit, so the producer serializes and
size-checks the complete begin frame before sending it. `data_base64` uses canonical RFC 4648
base64, including required padding and zero pad bits. A receiver checks the encoded chunk length
and base64 form before decoding, then checks that the decoded chunk is non-empty and no larger than
8,192 bytes before allocating blob storage. Empty blobs have no chunk requests and use the
canonical SHA-256 of zero bytes in `finish_blob`.

`closure_digest` is SHA-256 over the domain bytes `STS2/EXACT-RESTORE-CLOSURE/v1\0`, then for
each included item its unsigned 64-bit big-endian length and bytes. Order is the manifest, the
canonical payload, then distinct restore artifacts in first-reference order, excluding any
restore-artifact alias of the canonical payload or an earlier restore artifact. The manifest is
included separately once and is never deduplicated against blob bytes. A native receiver
recomputes this digest from staged bytes. Each response's `request_digest` is SHA-256 over the RFC
8785 canonical encoding of the complete current request frame, including its message and
correlation IDs; it is not the duplicate-begin identity. `receipt_digest` is SHA-256 over the RFC
8785 canonical encoding of the receipt object with `receipt_digest` omitted.

## Operation behavior

`begin` creates a new `STAGING` operation. A repeated begin with the same complete immutable
payload returns `EXISTING` and the actual current operation state; message and correlation IDs are
per request and do not change that identity. The immutable payload includes the operation and
current owner, selected branch and metadata revision, checkpoint and closure digests, manifest and
artifact references, compatibility, coverage, and boundary. Reusing the operation ID with any
changed immutable payload is rejected as `operation_conflict`. A duplicate never resets staging or
resumes a host effect. Before accepting a closure, the native owner must establish that a restore
adapter is available. A known unsupported adapter returns `REJECTED/no_restore_adapter` before
staging or byte upload; it must not fabricate `UNKNOWN`.

Chunk offsets are contiguous and ordered. An identical duplicate at the same offset is idempotent;
a changed duplicate, gap, overlap, bad digest, wrong size, wrong owner, or wrong phase is rejected.
`finish_blob` checks the complete blob size and digest. The operation reaches `CLOSURE_VERIFIED`
only after the manifest and every expected blob have been received and checked. A per-artifact
lookup reports its digest, total size, next offset, and verified flag; an operation-only lookup
reports bounded phase state without a list of blob progress.

`commit` is the sole host-effect request. Before calling a host adapter, the native owner durably
records commit intent. A verified receipt binds operation, selected child, current destination
owner, checkpoint, exact-state, manifest and closure digests, compatibility and coverage, boundary,
and the independently recaptured exact-state digest. `RESTORE_VERIFIED` requires that recaptured
digest to equal the selected checkpoint's exact-state digest.

Repeated commit requests return the stored phase or receipt and never invoke the effect again.
After commit intent, a process-loss or uncertain result is reconciled with `lookup`; neither
`COMMIT_INTENT` nor `UNKNOWN` authorizes another host mutation. If a request cannot be answered,
the typed `UNAVAILABLE` response distinguishes `host_effect=not_started` from
`host_effect=may_have_started`. `REJECTED` and `STALE_OWNER` always state `not_started`.
`NOT_FOUND` is a lookup result for an operation with no retained record.

An instance permits at most one unfinished operation; `UNKNOWN` counts as unfinished. Capacity
refusal happens before staging or effects. Up to 256 terminal receipts are retained without
eviction to permit a duplicate operation. Expiry may remove only `STAGING` or
`CLOSURE_VERIFIED` operations that never recorded commit intent; it never removes `COMMIT_INTENT`
or `UNKNOWN`.

This profile defines data and safety semantics only. It does not authorize a host effect, install
an adapter, authenticate a caller, expose an MCP tool, define a Gateway path, or claim native
restore support. `sts2-game-mod` must validate the real closure and recaptured state at its native
boundary before any consumer can describe restore as supported. Synthetic conformance and
recording-applier checks are not native restore evidence. A consuming wrapper binds its configured
principal and capability, message correlation, and exact expected-owner tuple on every phase; it
validates the returned correlation and owner before advancing. Those wrapper and transport rules
belong to the owning consumer, not this protocol artifact.
