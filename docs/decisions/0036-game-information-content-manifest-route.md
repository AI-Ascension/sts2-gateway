# ADR 0036: game-information content-manifest route

Accepted for the bounded whole-manifest gateway transport; native producer bytes, the MCP hop, and
deployment remain unverified.

## Requirement and ownership

`game-information-content-manifest-v1` carries one complete catalog for a configured content
authority, so a consumer that has resolved a query-shaped read can also obtain the catalog those
answers are scoped against. The producer side is already done: `sts2-game-mod` serializes the whole
manifest through `GameInformationContentManifestV1Codec` and serves it from its fixed owner route.
Gateway and MCP were the missing hops, so no authenticated, fenced surface exposed the manifest at
all. That absence is the blocker this change removes.

Gateway is a named consumer of the closed contract; the pinned manifest lists `sts2-game-mod`,
`sts2-gateway`, `sts2-mcp-server`, and `sts2-harness`. Gateway owns the fixed public route, caller
authentication, the instance and current lease fence, the admitted transport bound, and validation
of the returned envelope. The game-mod owns manifest contents, `inventory_revision`, and canonical
serialization. Gateway never fabricates, extends, reorders, repairs, or retains catalog content.

The copied artifact is pinned at schema digest
`416a39769445e6e462c5d5b5504f29010c255e2116a73094e55c7268e47f2ba6`, provenance artifact
`sts2-protocol/game-information-content-manifest-v1`, source
`schemas/game-information-content-manifest-v1.schema.json`, and generator `hand-authored`. Its
inventory is verified by `protocol-artifact/game-information-content-manifest-v1/SHA256SUMS`, and
the validating module re-derives the copied schema's own SHA-256 against the pin before trusting
it, so a drifted artifact fails closed rather than validating against a changed contract. The
manifest's `producer_integration: "pending in sts2-game-mod"` note is left as the historical record
it is; it is a manifest remark, not a pin, and it is not a fact this repository can maintain.
Contract changes require a new reviewed pin.

## Fixed route and admission

The public route is:

```text
GET /v1/instances/{instance_id}/game-information/content-manifest
```

It forwards only to the fixed producer path:

```text
GET /api/v1/game-information/content-manifest
```

The read is bodyless and selector-free: the envelope carries no `operation`, so it is admitted
exactly like the capabilities read rather than as one of the canonical query operations.
`GameInformationRoute::is_query()` is therefore false for it and the query payload validator never
runs, while the existing authorization and admission dispatch already key off
`GameInformationRoute::parse`, so the route inherits the whole game-information surface's existing
`Read` scope, authentication, instance fence, and caller-disconnect cancellation without a new
dispatch site. Any other method, a path under the same prefix that is not this operation, and a
foreign instance remain unrouted or rejected exactly as before.

## Bounded transport, and the refusal arm

The pinned profile permits a 16 MiB *message*, which is the producer's serialization ceiling. This
route's framing ceiling is the gateway's existing 128 KiB response bound, so the route admits a
gateway-bounded manifest rather than claiming to carry a message it cannot frame. That 128 KiB
figure is the admitted bound, not a truncation target.

A manifest cannot be shortened: a prefix of a catalog is a different, plausible catalog, and a
consumer that silently received one would make decisions against content the deployment never
published. So a producer exchange whose declared length exceeds the admitted bound is refused
from the declaration alone — before any producer byte is read — with HTTP `413` and the pinned
profile's own closed oversize arm:

```text
{"code":"result_limit_exceeded","reason":"serialized_payload_too_large"}
```

The refusal is an `error_response` envelope carrying `manifest: null`, the pinned protocol version,
schema digest, and provenance, and the caller's own correlation identity. It is not a producer
message and carries no manifest bytes; the caller's identity is interpolated into it only after it
has been proven to be the pinned schema's `id` token alphabet (`[A-Za-z0-9._:/-]`, at most 256
bytes), so no escaping question arises and the refusal path needs no JSON serializer.

## Pre-forward and post-forward checks

Before any producer connection, the route checks that the request body is empty, that the
correlation header `x-sts2-correlation-id` is present, and that its value is the admitted token.
A failure at any of those three points returns `400` with a named gateway code and performs zero
producer I/O.

After forwarding, gateway re-checks the current lease, re-checks that the game-information
authority is unchanged, and validates the whole exchange against the pin: body size, strict JSON
with no duplicate keys, `protocol_version`, `schema_digest`, exact three-member provenance, the
pinned schema itself, correlation equality between the envelope and the caller's header, the
envelope `kind`, and the kind/status pairing. `content_manifest_response` must carry status `200`;
`error_response` must carry a `4xx`/`5xx` status. A typed producer error keeps its status and body
verbatim and never produces a fabricated manifest.

## Revision fence

Query-v1 names the canonical `inventory_revision` in `content_manifest_id`. When the deployment
configures `STS2_GAME_INFORMATION_CONTENT_MANIFEST_ID` as a 64-hex digest, this route compares it
against the manifest's `inventory_revision` and refuses a difference with
`409 game_information_content_manifest_scope_rejected`, so a consumer cannot read catalog content
that the admitted query scope would reject. A configured label that is not a digest — the
component default `content-1`, for example — pins no revision and is not compared, because such a
label carries no digest claim to check.

## Rejection, cancellation, and outcomes

`400` for a body-bearing `GET`, a missing correlation, or a correlation outside the admitted
alphabet, each with zero producer forwarding. `413` with the protocol oversize arm for a declared
body beyond the admitted bound, and for an undeclared body that exceeds it after the fact. `409`
for a foreign pinned revision or a changed game-information authority. `499` for caller-disconnect
cancellation, reusing the existing game-information transport outcome. `504` on the exchange
timeout. `502` for malformed, duplicate-key, schema-invalid, status-inconsistent, or
correlation-mismatched producer responses. `503` when the producer is unreachable. Responses that
pass every check are relayed byte-for-byte with the producer's status.

## Deliberate omissions

This slice adds no operator enablement flag and no negotiated-capabilities offer. The
`negotiated-capabilities-v1` and `-v2` artifacts are checksum-verified closed contracts whose
`operation` enums are closed, so advertising a new operation would mean editing a pinned
associated artifact and every consumer's enum handling. That is a separate, reviewed change; until
then a consumer discovers this route from the gateway's own source and ADR rather than from an
offer.

## Compatibility and deterministic oracle

This is additive. The public route, downstream path, refusal arm, and pins are new; every existing
game-information route, protocol artifact, MCP frame, and game-mod contract is untouched, and no
existing response bytes change. An unconfigured or older deployment simply does not serve the new
path.

The deterministic oracle invokes the production dispatcher with a synthetic loopback producer and
checks route and method fixation, the absence of a query envelope, verbatim relay at the fixed
path with the forwarded identity headers, typed producer error relay without a fabricated
manifest, the declared-oversize refusal arm, admission and refusal of the pinned revision, a
foreign correlation, a drifted schema digest, an unrelated operation under the same prefix, the
three pre-forward rejections with zero producer I/O, an unreachable producer, and that the refusal
arm and admitted bound still match the pinned artifact.

This is gateway source/component evidence only. Native Mod production, MCP registration, Harness
use, deployment, and release compatibility remain separate evidence and are not established here.
