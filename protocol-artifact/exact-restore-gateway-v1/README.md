# `sts2-exact-restore-gateway-v1`

This MCP-owned wrapper binds a neutral `sts2-exact-restore-v1` frame to the configured harness
principal and the Gateway's `exact_restore` capability. It is a distinct consumer contract; it does
not modify the shared neutral protocol.

The request wrapper uses `actor.role: "harness"` and the response wrapper uses
`actor.role: "gateway"`. Both carry an `auth` object whose principal matches the actor, whose
capability is `exact_restore`, and whose `proof` is `null`. The MCP executable compares request
principal fields with configured `STS2_CALLER_ID`; callers cannot grant themselves a capability.
Gateway authentication uses the configured `STS2_RECOVERY_TOKEN` bearer and the fixed
`x-sts2-recovery-capability: exact_restore` header.

`payload.frame` is the complete neutral request or response frame. The request wrapper schema
admits only request kinds, and the response wrapper schema admits only response and error kinds.
The outer and inner
`message_id` and `correlation_id` values must match. A response correlation must name the current
request message ID. The neutral frame is validated against
`sts2-protocol/exact-restore-v1` schema digest
`2289d888c33eac46873408303c4423eab762e3f7bd6132ae8ae88d0d3b1858e4`; its `request_digest`
remains the digest of the canonical neutral request frame, not this wrapper.

Each full wrapper and each inner neutral frame is limited to 16 KiB. The adapter enforces the
neutral protocol's 8 KiB decoded chunk and 10,924-byte base64 limits before forwarding. The fixed
Gateway routes are `POST /v1/exact-restore/{begin,chunk,finish,commit,lookup}`. No route proxies an
MCP request or accepts a caller-selected path.

This is source and synthetic boundary evidence only. It does not establish Gateway deployment,
native staging, host restore, or post-restore recapture. In particular, advertising this MCP tool
profile does not claim a native restore adapter is available; Gateway's known
`REJECTED/no_restore_adapter` result is preserved before any upload phase.
