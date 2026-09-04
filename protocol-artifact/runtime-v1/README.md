# `runtime-v1` release-like artifact

This artifact defines the first live vertical-slice wire contract. It is deliberately limited to
an authenticated state snapshot, the safe `show_runtime_probe` host action, and a fresh observation
with a visible status-overlay effect witness. It does not define gameplay rules, saves, process
ownership, MCP framing, or gateway authentication.

The schema source is [`schemas/runtime-v1.schema.json`](../../schemas/runtime-v1.schema.json), and
`schema.json` is its byte-identical package copy. The schema digest in `manifest.json` identifies
the exact source bytes. The contract is release-like evidence for the named consumers, not a public
release or a claim of host compatibility by itself.

The golden messages cover the initial read, action request, accepted effect witness, and stale
generation rejection used by the cross-repository slice.
