# Compatibility policy

## Current evidence

The `protocol-artifact/poc-v1/` directory is an offline release-like copy consumed by the POC
test. Exact artifact identity and fixture bytes are confirmed locally; this does not establish
compatibility with a live game-mod, host, network, or runtime.

The copy is synchronized verbatim from the normative protocol artifact at source revision
`cad3c85d3cba3363ad387f9c26a3c3cac2782267` (protocol PR #2). Its manifest binds the package
schema path to the canonical source digest, and the gateway's copied source/package paths are
verified separately.

The Runtime-v2 copy is synchronized from the protocol handoff at commit
`8d4b2f574cf860a71f2a5e4ce3308ac069cb1527`. Its source and package schema bytes both have digest
`f7963b19c8ed5bbdc02c08e83c7a2e16c4771ed5eb798b29a8208d7a917a86c2`, and the local
`protocol-artifact/runtime-v2/SHA256SUMS` inventory passes. This proves artifact-copy integrity,
not consumer or host compatibility.

This target has repository governance, one target-owned control-plane package, a separate attached
runtime binary, and deterministic fake tests. The controlled component lane uses synthetic data,
while the authorized exact-host lane uses the packaged game-mod listener. Attached forwarding,
authentication, lease fencing, and the bounded probe path are confirmed for that exact host; external
process supervision, real concurrency isolation, host gameplay, and general compatibility remain
`unverified`.

Static policy results may establish configuration and source compatibility with the pinned Rust
toolchain. They do not establish compatibility with a game or a historical implementation.

The Runtime-v1 inert copy now includes the canonical checksum inventory, five golden messages,
source schema and conformance companion from merged protocol `main`. Existing package schema and
manifest bytes are unchanged; the README restores its canonical local source link. CI verifies
both frozen Runtime-v1 and Runtime-v2 inventories. This confirms copy integrity only.

## Independent version axes

Keep these values separate and record each in a future compatibility matrix:

| Axis | Owner | Compatibility question |
| --- | --- | --- |
| Rust/toolchain | repository | Can the governance and gateway code build with the declared MSRV? |
| Gateway API | gateway | Do control/data requests, errors, identity, and lease rules match? |
| Game-mod HTTP contract | game-mod | Does the fixed downstream route contract match? |
| Game host/loader | game-mod | Can the host boundary load and execute in the claimed environment? |
| MCP revision | MCP server | Can the adapter map its accepted calls to the gateway? |
| Harness client | harness | Can the coordinator preserve instance and lease lineage? |
| Shared protocol | protocol owner | Do neutral contract versions and fixtures remain compatible? |

One axis must not be inferred from another. A host update does not automatically change the gateway
API, and a successful gateway acknowledgment does not prove game state or effect settlement.

## Change classification

The ledger's monotonic-observation repair is a patch to the existing freshness invariant; it does
not change Runtime-v2 artifact bytes. Historical receipts keep their own generation and are distinct
from the newest admission observation. Corrupt/inconsistent checkpoint generations fail closed.

The generic control-plane recovery correction is a patch to failed-start capacity and expiry error
reporting, not a new route or wire field. Failed starts now leave no queryable allocation because no
allocation identity was returned; consumed IDs are not reused. Expiry reconciliation returns the
existing `ProcessStop` error on failed cleanup instead of incorrectly reporting `Expired`. Callers
must retain the already-issued allocation identity and invoke authorized `cleanup` after the fault is
resolved. `ProcessPort::start` transfers a handle only on success; partial-start cleanup on error is
the port's responsibility. No concrete process adapter is validated by this correction.

- **Patch:** correction that preserves accepted identity, route, lease, error, and timing behavior.
- **Minor:** additive bounded field or operation with an older-client behavior defined.
- **Major:** changed lifecycle state, route/method, auth scope, lease/fence rule, error semantics,
  timing guarantee, or removal requiring migration.

Every public change needs a requirement, deterministic conformance case, migration note where
needed, and an updated compatibility record. Boundary-specific behavior remains local unless a
versioned, neutral protocol contract is accepted by its named consumers.

## Evidence record

Future records must include exact target revision, toolchain, OS/architecture, game/mod/host versions
when applicable, contract digests, instance and lease identities, clock/seed, disposable fixture
status, sanitized commands/logs, and evidence level. Use `confirmed` only for an authorized controlled
test; use `source-derived`, `inferred`, `proposed`, or `unverified` precisely.

## Runtime adapter row

The independently merged [ADR 0011](decisions/0011-attached-runtime-hardening.md) baseline is retained:
configured loopback ports must be nonzero, allocation uses a closed typed body (missing/unknown/
duplicate fields return400), and same-process reallocation after release returns409
`lease_context_revoked`. Component additions in ADRs 0007–0010 remain implemented; merging the
baseline does not replace concrete v2 forwarding with its earlier unconfigured adapter.

Endpoint configuration now enforces numeric loopback socket addresses (`127.0.0.1:port` or
`[::1]:port`). Previously accepted DNS names, wildcard binds, and remote addresses must be migrated
to an explicit numeric loopback endpoint; this enforces the documented local-only trust boundary.
Attached action and receipt routes restrict operation IDs to 1–128 ASCII letters/digits or `-_.:`,
without `..`. Slash-containing operation IDs allowed by the neutral contract cannot occupy one
fixed route segment, so action admission rejects them before dispatch. This matches MCP PR #7's
route profile without changing frozen Runtime-v2 schema bytes or generic ledger identity rules.
Release/shutdown now permanently revoke the attached configured lease for that process lifetime.
Clients cannot allocate the same context again to undo revocation; a coordinator must provide a
fresh session/lease/epoch for replacement ownership. This does not implement durable restart fencing.
The independent Runtime-v2 split preserves the frozen artifact and fixed v2 routes. The Exo
Runtime-v3 profile is integrated separately after its protocol dependency is accepted.

| Adapter | Downstream | Current evidence | Result |
| --- | --- | --- | --- |
| `sts2-gateway-runtime` | Attached loopback runtime-v1 listener | Rust gates, synthetic TCP lane, and authorized exact-host trace | Attached forwarding and lease path confirmed for STS2 v0.107.1 Windows x86-64; general lifecycle and gameplay unverified |
| Runtime-v2 ledger and attached adapter | Owner-local ledger plus fixed synthetic TCP downstream | Rust gates, byte-level artifact verification, deterministic fault tests, and isolated component restart trace | Fixed state/action/operation forwarding, bounded optional journal recovery with exclusive path ownership, exact bearer check, and synthetic route behavior confirmed; live downstream action settlement, lease-epoch rotation, multi-instance isolation, and host compatibility unverified |

The adapters' fixed configurations are sprint boundaries, not general lifecycle support claims. The
attached Runtime-v2 process accepts an optional bounded version-1 journal and a retained-operation
capacity of 1 through 64. The service owns an exclusive stable lock sibling for the configured
journal path and fails closed when another process already holds it; each instance must use a distinct
path. A journal identity or lease-epoch mismatch fails closed; an in-flight or
accepted operation restored after restart becomes explicit `unknown` and is reconciled read-only.
Clients must not blindly resend an unknown action. A future compatibility promotion must add exact
process ownership, readiness, lease-epoch rotation, multi-instance, downstream crash, and
disposable-host evidence. The attached process also accepts a bounded FIFO queue-capacity setting
from 1 through 64, exposes sanitized metrics, and supports a lease-fenced shutdown route. These
additions are component lifecycle controls; they do not establish process ownership, signal
handling, global scheduling, or host compatibility. `STS2_MCP_SESSION_ID` defaults independently to `mcp-session-1`, matching MCP and harness; the gateway
session remains `session-1` by default and may be set independently; every lease-protected request must then carry the matching
`x-mcp-session-id` value.

## Runtime-v3 and co-op row

| Surface | Current evidence | Result |
| --- | --- | --- |
| `runtime-v3-gameplay` fixed routes and forwarder | Source validation and route allowlist tests | Source-derived; live gateway/host settlement unverified |
| Co-op synchronization and process supervisor | Bounded deterministic fakes and identity/failure tests | Source-derived; live restart, cleanup, isolation, and multiplayer unverified |

These surfaces are additive to Runtime-v2 and do not inherit its runtime evidence.

The gameplay envelope is pinned to protocol PR #8 commit
`82507361890c1bdce6cffeaf7e616d93e53a7d99`, schema digest
`b37c80f583aeaf4f81ede2083bcfb4129196baf5eb092470e8738173c4b7226c`.
The complete copied artifact and its source/conformance companions are checked by CI. Runtime
validation additionally enforces duplicate-field rejection, schema shape, byte bounds, correlated
identities/operations, and observation/witness relationships. This is the semantic gameplay
profile, not the incompatible earlier bounded-card profile used by gateway PR #6. The same
profile name does not establish compatibility; the exact digest is required and mixed digests
are rejected. See [ADR 0014](decisions/0014-runtime-v3-framing-and-fencing.md).

The attached executable has a boolean active lease, **not** a timed/renewable lease. It has no
durable boot-epoch rotation. Starting another process with the same configured identity/token/
epoch and allocating it can admit proofs from the earlier process; this remains an unresolved
deployment blocker. Release cannot reactivate the context within one process, but its revocation
is not durable. The injected process supervisor and generic clock-based core do not change these
executable semantics. A compatible design must separate historical read-only receipts from a
fresh active boot context across gateway, mod, MCP, and harness before restart can be safe.

## Attached runtime hardening compatibility

[ADR 0011](decisions/0011-attached-runtime-hardening.md) corrects the existing unpublished attached
baseline without adding a gameplay profile, journal, queue, or restart issuer. The component
ADRs add the v2 journal and queue above this baseline. Both configured
addresses must now be literal loopback addresses with nonzero ports. Clients using DNS names or
non-loopback endpoints must change configuration. Incoming reads and reply writes each expire
after five seconds; the whole downstream connect/write/read exchange shares five seconds. An
incomplete inbound request expires with HTTP408; an uncertain downstream mutation is not retried.
The existing body/response limits and frozen Runtime-v2 artifact are unchanged.

That statement describes the inherited v1/v2 hardening. The separate semantic Runtime-v3
extension uses a128KiB parsed-response limit for larger player-visible observations; its regression
checks that bound. The legacy v1 relay still imposes its16KiB returned-body limit. Neither bound
permits unbounded host payloads or changes the frozen Runtime-v2 artifact.

Allocation uses a closed typed body: malformed, duplicate, missing or unknown fields return400;
well-formed requests naming the wrong configured identity return409. No partial allocation occurs.

Release is terminal for the fixed context in that process; reallocation returns409
`lease_context_revoked`. This is an intentional compatibility correction to unsafe lease reuse,
not automatic epoch rotation. The binary still uses a boolean active lease, with no TTL/renewal,
durable boot epoch, or persisted revocation. Restarting with the same config and allocating can
still admit earlier proofs; no restart-ready or autonomous-gameplay claim follows from these fixes.
Runtime-v2 exact receipt replay is read-only and need not match current state generation, but it
still requires the original identity/epoch and canonical payload; fresh mutations remain fenced.

## Runtime-v2 required nullable members

This patch enforces the existing frozen Runtime-v2 schema: nullable members must be present, even
when their value is null. Clients omitting a member must send an explicit null instead. No route,
field, digest, artifact byte, or valid serialized message changes.

## Exo component integration

Runtime-v3 inherits the component adapter authentication settings and configured MCP session
header. Read scope authorizes state, legal actions, wait, and reobserve; mutate scope is required
for dispatch, and control scope for recovery. A read credential cannot dispatch or recover.
Co-op and process-supervisor APIs remain local prototypes: the attached runtime does not consume
co-op wire messages or use peer synchronization to authorize forwarding.

Recovery uses route-level control scope because the canonical request kinds include release-lease
and stop-episode as well as reobserve and reconcile. The current mod rejects those lifecycle
kinds as unsupported, but the gateway does not weaken their future authority. Read-only clients
can use the dedicated reobserve and wait routes for state and retained receipt observation.
