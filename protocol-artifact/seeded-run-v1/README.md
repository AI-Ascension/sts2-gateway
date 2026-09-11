# `seeded-run-v1` release-like artifact

This artifact defines a bounded, explicitly seeded launch operation. The harness chooses the
requested seed; the game host remains authoritative and must read back the canonical seed. An
`accepted` result is admission only. A `settled` result requires a fresh host observation and the
`run_started` effect witness. An `unknown` result must be reconciled by the same operation ID.
Every start request and result carries `selected_context`, which binds the native standard mode,
Ironclad, ascension, modifiers, ordered acts, selection policy, profile baseline, save policy, and
separate game/mod compatibility identities. Its `context_digest` is the SHA-256 of those fields in
compact canonical JSON, excluding the digest member itself. A reconciliation request remains
read-only and carries no launch context.

The normative source is [`schemas/seeded-run-v1.schema.json`](../../schemas/seeded-run-v1.schema.json),
and `schema.json` is its byte-identical package copy. The manifest digest identifies the exact
source schema bytes. `SHA256SUMS` covers the source, package, manifest, goldens, and conformance case.

The vectors are deterministic contract fixtures. They do not prove that a consumer, game host,
gateway, MCP server, harness, or live gameplay installation is compatible.
