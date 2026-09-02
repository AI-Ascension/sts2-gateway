# `poc-v1` release-like artifact

This directory is the checked-in, offline artifact consumed by the five POC owners. The schema
source is [`schemas/poc-v1.schema.json`](../../schemas/poc-v1.schema.json), and `schema.json` is a
byte-identical package copy. The manifest's `schema_digest` identifies those exact schema bytes;
`SHA256SUMS` covers the source, package, manifest, fixtures, and conformance case. The artifact is
not a public release and does not establish game, host, network, or runtime compatibility.
