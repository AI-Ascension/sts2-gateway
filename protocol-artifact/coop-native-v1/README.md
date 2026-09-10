# `coop-native-v1` component contract

This directory records the accepted source and serialization component contract for native
co-op actuation. It contains the exact schema, producer projections, seventeen strict golden
envelopes, and boundary conformance metadata. The contract covers observation, a host-backed
legal catalog, local actions, shared votes, peer rejoin, effects, receipts, and same-operation
recovery.

The managed producer at source commit
`ab702dbbc79bc5854bd0840b44a729834ae50e68` (tree
`e3f0aa9d30fe25585fbb3c1fe8e3c1fcdfa43223`) declares the exact schema digest
`2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629`. The producer capture is
source-only: `CapturePort` is synthetic and does not load STS2, connect native peers, or prove a
live host outcome. The goldens are compact projections of the checked-in capture wrapper members,
including explicit nullable envelope fields and receipt evidence.

The component contract registers `sts2-gateway`, `sts2-mcp-server`, and `sts2-harness` as
boundary consumers. Each consumer must copy the exact artifact, preserve the closed JSON shape,
and keep host authority, gateway routing, MCP framing, and harness coordination in its owning
repository. Unknown members, duplicate keys, unsupported kinds, stale generations, and
unfenced identities remain errors; an `unknown` outcome is retained for same-operation recovery
and never retried as a new mutation.

Component acceptance does not establish live native support. The separate runtime gate still
requires a disposable two-peer native STS2 session with distinct peer identities, a settled
model action and supported shared votes, matching native checksums, disconnect/rejoin recovery,
and peer convergence. Until that evidence exists, `live_status` remains `unverified` and this
artifact must not be described as a live multiplayer release.
