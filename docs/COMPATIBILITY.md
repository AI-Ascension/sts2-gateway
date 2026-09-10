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

The T12 workflow authority contract is an additive gateway-local source/component surface. A
workflow ledger must be constructed with an owner authority and recovery capability contract, and
its mutation, state-refresh, cancellation, and retained-receipt paths require the matching identity
and boot epoch. The existing `RuntimeV2Ledger::new` component lane remains compatible; implicit
workflow calls on a recovery ledger reject with `AuthorityRequired`. Missing or changed boot
identity in workflow state rejects restoration with `PersistedStateMismatch`, and unavailable
receipt retention rejects reconciliation before any receipt read. No Runtime-v2 artifact, MCP route,
protocol/mod file, or attached executable restart guarantee changes.

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
Runtime-v3 profile is integrated at the current gateway main against the accepted protocol profile;
its source/component boundary remains separate from native host legality, provider execution,
deployment, and release evidence.

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

## Runtime-v4 expert source/component row

The additive `runtime-v4-expert` surface is implemented at the gateway source/component boundary
at current main commit `2b44bf347f790509c9f13378c89719d09366d45b`. The gateway admits fixed expert-state,
expert-action, and expert-reconcile paths, validates the request/response envelopes, and forwards
only the corresponding fixed paths to the attached mod boundary. Its copied artifact identities
are:

| artifact | schema digest | checked-in location |
| --- | --- | --- |
| Runtime-v4 expert observation | `0ee034d5da83f34e9fa0ba23038738d56ef8cfccb1c6e752af3ab63d212c8e42` | `protocol-artifact/runtime-v4-expert/schema.json` |
| Runtime-v4 expert action | `393318bda8c3522c0ecbacc78b95471a9f4dc3f825169d2048f4c74a7b7f2929` | `protocol-artifact/runtime-v4-expert-action/schema.json` |

| Surface | Current evidence | Result |
| --- | --- | --- |
| `runtime-v4-expert` state/action/reconcile routes | Source validation, strict envelope checks, artifact checks, and gateway workspace policy/tests/Clippy at current main `2b44bf3` | Source/component confirmed; native host legality, settled host effects, provider runs, deployment, and broader compatibility unverified |

The source/component checks do not establish a running mod, a valid host observation, a settled
potion effect, a model-controlled episode, or compatibility with an arbitrary host version.

## Seeded-run gateway row

The additive `seeded-run-v1` seam is implemented at current gateway main
`2b44bf347f790509c9f13378c89719d09366d45b`. It exposes fixed instance-scoped
`POST /v2/instances/{instance_id}/seeded-run` and bodyless
`GET /v2/instances/{instance_id}/seeded-operations/{operation_id}` routes. The gateway validates
the complete selected context, its content-addressed digest, operation and correlation identity,
lease/epoch fence, and bounded response before forwarding only the fixed mod paths.

The ledger distinguishes accepted, settled, rejected, cancelled, and unknown outcomes. A timeout
or transport uncertainty stays unknown and is reconciled read-only by the original operation
identity; it is never retried as a new seed mutation. The optional journal sidecar restores only a
matching operation and binding after restart. The copied artifact is schema digest
`5c659f344be78f84e8d783986925d462714f933cac95d18943358992f7d3e2b8`, aligned with protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`. These are source/component and artifact-copy checks;
native host settlement, save/profile isolation, gameplay, deployment, and release compatibility
remain unverified. See [ADR 0018](decisions/0018-seeded-run-v1-gateway-boundary.md).

## Runtime-v4 expert rest-action candidate row

The gateway implements the HTTP assignment for the candidate
`runtime-v4-expert-rest-action-v1` profile. The copied artifact remains `candidate` with an empty
admitted-consumer list; the exact identity is:

| artifact | schema digest | checked-in location |
| --- | --- | --- |
| Runtime-v4 expert rest action | `bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd` | `protocol-artifact/runtime-v4-expert-rest-action/schema.json` |

| Surface | Current evidence | Result |
| --- | --- | --- |
| `runtime-v4-expert-rest-action` dispatch/reconcile routes | Fixed route and method tests, exact request/response artifact validation, 16 goldens, 22 mutation rejections, and serialized Smith/Mend producer-shaped lifecycle checks | Source/component confirmed; native producer, host legality/effects, MCP/harness consumers, deployment, and release unverified |

Dispatch is `POST /v4/instances/{instance_id}/expert-rest-action` with mutate scope and JSON
content type. Reconciliation is `GET /v4/instances/{instance_id}/expert-rest-actions/{operation_id}`
with read scope and an empty body. Both retain the existing lease and identity fences and forward
only the fixed downstream paths. The consumer retains a bounded selector-admission catalog across
responses; a completed selection with no prior valid catalog, an unlisted card/player, an invalid
selector kind/count/catalog, or an inconsistent effect witness is rejected before the response is
returned. A caller timeout or disconnect never retries a mutation; recovery uses the original
operation identity. The profile is additive and does not alter Runtime-v1 through Runtime-v4 expert,
map, co-op, or legacy routes.

### Historical Runtime-v4 settlement-fencing update — 2026-09-07

The prior Runtime-v4 source/component record at commit `17b93bf35e5256f6adf690aa148fa57d4f56c523` remains retained above as the earlier evidence. The exact-head update at `aecc9fa44c825623b3e3bbb21d130e1fe6ac9468` binds nested settled observation `state_id` and `generation` to the outer response, and binds dispatch transition `before_generation` to the request generation.

| Surface | Current evidence | Result |
| --- | --- | --- |
| `runtime-v4-expert` settlement fencing | Exact-head source/component review; 130 workspace tests, strict policy, format, Clippy, and three original regression cases passed | Source/component confirmed at `aecc9fa`; native host legality, settled host effects, provider execution, cross-consumer integration, deployment, and release remain unverified |

## Runtime-v3 and co-op row

| Surface | Current evidence | Result |
| --- | --- | --- |
| `runtime-v3-gameplay` fixed routes and forwarder | Source validation and route allowlist tests | Source-derived; live gateway/host settlement unverified |
| Older numeric-ID co-op and process-supervisor prototypes | Bounded deterministic fakes and identity/failure tests | Source-derived; live restart, cleanup, isolation, and multiplayer unverified |
| Coordinator-reported synchronization | Real gateway/MCP executable exchange plus injected-time ledger tests | Confirmed coordination component; native multiplayer unverified |

These surfaces are additive to Runtime-v2 and do not inherit its runtime evidence.

The gameplay envelope is pinned to current protocol main
`d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`, schema digest
`8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63`.
This coordinated profile adds argument-free proceed, confirm-selection and cancel-selection
actions. Producer and all consumers must migrate together; earlier digests are rejected.
The complete copied artifact and its source/conformance companions are checked by CI. Runtime
validation additionally enforces duplicate-field rejection, schema shape, byte bounds, correlated
identities/operations, and observation/witness relationships. This is the semantic gameplay
profile, not the incompatible earlier bounded-card profile used by gateway PR #6. The same
profile name does not establish compatibility; the exact digest is required and mixed digests
are rejected. See [ADR 0014](decisions/0014-runtime-v3-framing-and-fencing.md).

Runtime-v3 envelope validation depends on the `jsonschema` crate, pinned to `=0.52.1` with default
features off (no remote or filesystem reference resolution). The validator is compiled once from
the embedded schema; a unit test proves the embedded schema compiles and admits a golden request so
a silent fail-closed rejection of every v3 envelope cannot go unnoticed. The workspace lint
`unsafe_code = "forbid"` is unchanged for all first-party crates. Acceptance conditions and the
evidence for each are in [ADR 0015](decisions/0015-jsonschema-dependency-acceptance.md).

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
The T12 workflow contract rejects stale or missing owner boot identity at its gateway boundary but
does not alter the attached executable's restart behavior.

## Runtime-v2 required nullable members

This patch enforces the existing frozen Runtime-v2 schema: nullable members must be present, even
when their value is null. Clients omitting a member must send an explicit null instead. No route,
field, digest, artifact byte, or valid serialized message changes.

## Exo component integration

Runtime-v3 inherits the component adapter authentication settings and configured MCP session
header. Read scope authorizes state, legal actions, wait, and reobserve; mutate scope is required
for dispatch, and control scope for recovery. A read credential cannot dispatch or recover.
The process-supervisor and older numeric-ID co-op APIs remain local prototypes. The attached
runtime separately produces the complete `coop-synchronization-v1` response from recent
coordinator reports, with no use of peer synchronization to authorize gameplay forwarding.

Recovery uses route-level control scope because the canonical request kinds include release-lease
and stop-episode as well as reobserve and reconcile. The current mod rejects those lifecycle
kinds as unsupported, but the gateway does not weaken their future authority. Read-only clients
can use the dedicated reobserve and wait routes for state and retained receipt observation.

## Opt-in coordinator reports

With `STS2_COOP_ROSTER` configured, the exact attached instance gains two routes:

| Method/path suffix under `/v1/instances/{id}` | Scope | Behavior |
| --- | --- | --- |
| `GET /coop/synchronization` | read | Empty body; pinned complete synchronization response |
| `POST /coop/peer-report` | control | Closed `{peer_id, generation, connected}` report; no host call |

Both require the normal active lease, caller, separate gateway/MCP sessions, epoch and
correlation headers. Roster entries are closed `{peer_id, role}` objects, two to four peers,
exactly one local. IDs are unique bounded ASCII; roles are `local` or `ally`. Missing config
returns 503; malformed input 400; unknown peers, generation rollback or inactive/stale binding
409; authentication/scope rejection remains 401/403. Lease release and shutdown fence both.

All peers start missing. Connected reports expire at thirty seconds on the monotonic clock;
the coordinator must refresh every peer before that deadline. Generation advances only when
all recent connected reports agree, never backwards. These sequences belong to coordinator
convergence, not independently allocated per-game observation counters. The wire source is
always `gateway_peer_reports`; no independent peer authentication or native-host agreement is
claimed. No state is persisted or restored as synchronized after process restart.

## Runtime-map visibility row

| Surface | Producer pin | Current evidence | Result |
| --- | --- | --- | --- |
| `runtime-map-v1` artifact and schema | current protocol main commit `d3ab5fca7d9d74bb31eeb3e5b343d8024ee44404`, schema digest `ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b` | copied manifest, schema, conformance case, and three goldens with checksum validation | Source-derived artifact-copy integrity at current gateway main `2b44bf3`; producer and host compatibility unverified |
| Gateway map snapshot route | `GET /v1/instances/{instance_id}/map-snapshot` to fixed downstream `GET /api/map/v1/snapshot` | exact route, lease/bodyless admission, schema/provenance/identity/generation checks, bounded graph and binding tests | Confirmed deterministic source/component behavior; live map observation and freshness unverified |

The profile is additive and does not alter legacy Runtime-v1, Runtime-v2, Runtime-v3 gameplay, or
coordinator-report routes. It carries visible nodes, directed edges, position, history, terminal
references, and navigation bindings only. Hidden host state is outside the contract. Overlapping
coordinates and disconnected visible components are preserved. The consumer rejects mixed
protocol revisions, wrong schema digests, foreign or stale generations, malformed graphs, invalid
action-option identity, and responses over 256 KiB; it does not retry or synthesize a map snapshot.

## Proposed retained receipt query

The proposed `coop-receipt-query-v1` profile adds one read-only route:

| Method/path suffix under `/v1/instances/{id}` | Scope | Downstream path | Status |
| --- | --- | --- | --- |
| `POST /coop/receipt-query` | read | `POST /api/v1/coop/native/receipt-query` | proposed, unadmitted |

The route requires the existing authentication, caller, instance, session, MCP-session, active
lease, epoch, and correlation fences. Before any downstream connection, it bounds the request,
requires JSON content type, and validates the copied profile at schema digest
`3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c`, including exact provenance,
canonical member order and UTF-8 encoding, duplicate-key rejection, repeated identity, sorted
distinct participant IDs, and request generation lineage. The response must repeat the request
identity and use `evidence_scope: retained_receipt`; accepted, settled, and rejected receipts
must match their status, while unknown and recovery-required responses carry no receipt.

The route forwards the validated neutral bytes to the fixed game-mod path and validates the bounded
response before returning it. It never observes, reconciles, queues, retries, or authorizes a
mutation. Invalid request data returns a client error; a downstream failure is returned as an
unavailable error; an invalid or oversized downstream response is a `502`. The profile manifest
intentionally keeps `consumers: []`; the route is source/component evidence only until the mod
producer, MCP reader, harness recovery reader, and independent admission review are complete.
