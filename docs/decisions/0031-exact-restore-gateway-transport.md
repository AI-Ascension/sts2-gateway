# ADR 0031: Fixed Gateway transport for exact restore

## Status

Accepted for the Gateway source/component boundary. This decision adds authenticated transport
routes only; it does not claim that a native restore adapter is available.

## Context

The neutral `sts2-exact-restore-v1` protocol defines bounded closure transfer and operation
receipts. Gateway owns the current instance lease and host-installation fence, so an exact-restore
request must be checked against that live owner on every phase. Gateway forwards a validated
neutral frame only to its matching fixed native path and does not route back through MCP.

The MCP-owned `sts2-exact-restore-gateway-v1` wrapper binds the configured Harness principal and
the `exact_restore` capability. Neither wrapper authentication nor a historical owner record
establishes current lease liveness.

## Decision

Add five fixed `POST` routes:

| Gateway route | Native path |
| --- | --- |
| `/v1/exact-restore/begin` | `/v1/exact-restore/begin` |
| `/v1/exact-restore/chunk` | `/v1/exact-restore/chunk` |
| `/v1/exact-restore/finish` | `/v1/exact-restore/finish` |
| `/v1/exact-restore/commit` | `/v1/exact-restore/commit` |
| `/v1/exact-restore/lookup` | `/v1/exact-restore/lookup` |

Every request requires recovery authentication with control scope, the
`x-sts2-recovery-capability: exact_restore` header, the configured Harness principal, the complete
closed wrapper and neutral frame, matching outer/inner message and correlation IDs, and the
configured instance/caller/session/MCP-session/lease/epoch headers. The Gateway correlation header
must equal the inner neutral request correlation ID. Request and response wrappers and neutral
frames are each limited to 16 KiB. Duplicate JSON keys and non-canonical serialized JSON reject.
Unknown methods and paths are not forwarded.

Before each forward and after receiving the native reply, Gateway rechecks the current durable and
process-held owner state, exact installed host grant, active in-memory lease deadline, and caller
lease headers. The neutral `expected_owner` must equal the complete currently available owner tuple.
Gateway forwards only the neutral frame to the fixed native path with configured native
authentication and the same correlation ID. It validates the complete neutral response, operation,
owner, request digest, response kind, and response correlation before returning it in the closed
Gateway wrapper. It never proxies MCP requests and never chooses a fallback destination.

Authentication or capability failures reject before owner reads. Missing, expired, unknown, or
foreign owner state rejects before downstream forwarding. An invalid native response becomes a
bounded gateway error. A downstream timeout remains unavailable; Gateway does not retry any
phase. Native `REJECTED/no_restore_adapter` is preserved as an explicit response with
`host_effect: not_started`. Harness must stop before uploading closure bytes after that result.

## Ownership and compatibility

Gateway owns these routes, the live-owner checks, fixed native forwarding, byte bounds, and
response validation. MCP owns the wrapper, configured principal/capability mapping, and tool
catalog. Harness owns closure verification and transfer orchestration. Game-mod owns native staging,
commit intent, restore effects, and post-restore recapture.

The five routes are an additive minor surface. Existing recovery and gameplay contracts are
unchanged. Clients that do not use `exact_restore` are unaffected. A route response from a Gateway
with no installed native restore adapter is not evidence that a checkpoint was restored.

## Deterministic oracle

Production-dispatch tests use a ready SQLite-backed Gateway fixture with an acknowledged host
grant and current in-memory lease. They exercise all five fixed routes through the real
`RuntimeService` dispatcher and a loopback native peer, checking exact path, neutral body,
correlation, identity headers, and wrapper response binding. Foreign owners, wrong capabilities,
duplicate JSON, stale host grants, unavailable owner state, wrong methods, and arbitrary paths
must result in zero native connections. These tests establish Gateway source/component behavior
only; native restore, process restart, host effects, MCP execution, and Harness end-to-end
continuation remain separate evidence.
