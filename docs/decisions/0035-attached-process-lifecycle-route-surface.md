# ADR 0035: attached process-lifecycle route surface

- Status: Accepted for the gateway source/component boundary
- Date: 2026-09-18
- Owner: `sts2-gateway`
- Requirement: issue #50
- Amends: [ADR 0024](0024-process-lifecycle-ownership.md)

## Context

[ADR 0024](0024-process-lifecycle-ownership.md) added a complete approved-profile lifecycle
coordinator: `ApprovedLaunchProfiles`, `ProcessLifecycle`, and the `LifecycleRecordStore` seam. It
deliberately stopped at the trait boundary and recorded the concrete OS process adapter as a
reviewed deployment input.

What it did not add was any way to *reach* the coordinator. `crates/gateway/src/bin/runtime_support/service_routes.rs`
had no dispatch for the lifecycle component at all, so an attached deployment could compose a
coordinator and still have no route that authenticated, fenced, and dispatched a lifecycle
operation. That route surface is the named contract gate for `sts2-gateway#50` items 2 and 4 and
for `sts2-harness#101`.

The route surface cannot simply forward the caller's identity fields, because the two identity
spaces disagree. The attached runtime authenticates requests with string instance/caller/session/
lease identities from configuration, while the coordinator's identity space is numeric
(`InstanceId`, `CallerId`, `SessionId`, `LeaseId` are all `u64`-backed) because its durable store
keys records by `i64`.

## Decision

Add three fixed, lease-fenced, authorization-scoped routes to the attached runtime:

| Method | Path | Scope | Effect |
| --- | --- | --- | --- |
| `POST` | `/v1/instances/{instance}/process-lifecycle/operations` | `Mutate` | Submit one lifecycle action |
| `GET` | `/v1/instances/{instance}/process-lifecycle` | `Read` | Advertise contract, profiles, authority epoch |
| `GET` | `/v1/instances/{instance}/process-lifecycle/operations/{id}` | `Read` | Look up one retained operation |

The wire contract string is `sts2-gateway-process-lifecycle-v1`.

### No caller-supplied identity

A submission body carries exactly three things: an opaque `operation_id`, an `authority_epoch`, and
one closed `action`. Instance, caller, session, lease, and lease epoch are read only from gateway
configuration. The action is a closed schema with `deny_unknown_fields` and a per-action allowed
field guard, so a body cannot smuggle an alternate action or attach fields belonging to another
action. Executable, install, image, user-data namespace, and process policy are never expressible
on the wire: a caller supplies only an opaque `LaunchProfileId` that the server-owned catalog
resolves.

### Numeric bridge for the two identity spaces

String identities are mapped to the coordinator's numeric space by a domain-separated SHA-256
truncation: each namespace hashes `<domain> + "\0" + <value>` and keeps the leading 64 bits, with
the domain being `instance`, `caller`, `session`, or `lease`.

Two properties are load-bearing:

- **Deterministic across restarts and processes.** Durable operation records must keep matching
  their lease after a restart, so the mapping cannot depend on process-local state, a random seed,
  or insertion order.
- **Not caller-selectable.** The input is gateway configuration that the request has already been
  fenced against, so a caller cannot choose its own numeric identity.

The digest is then masked to 63 bits with the low bit forced on. The durable SQLite store keys
records by `i64`, so a derived value above the signed range cannot be persisted at all and would
surface as a store error rather than a fence decision. Masking keeps every derived identity
storable; forcing the low bit on keeps no identity equal to the reserved zero value.

Collision resistance is a convenience here, not the security boundary. A fence decision compares
the complete five-field proof, and every request must additionally pass the configured string fence
in `service_lease.rs`, so a forged match would have to collide in all five namespaces at once while
also presenting the correct configured strings.

### Server-owned catalog configuration

The launch-profile catalog is built only from gateway configuration
(`STS2_PROCESS_LIFECYCLE_PROFILES`), parsed as
`id:install:executable:image:namespace:descendants:start_ms:stop_ms` entries separated by `;`.
Every entry is resolved through the component's own validating constructors, so configuration
cannot admit a zero id, a zero identity field, or an out-of-range policy that the coordinator would
reject. An invalid entry fails startup rather than being skipped: a partially valid catalog would
silently change which profiles exist between restarts.

### Fail-closed default

Production composition validates the catalog, the capacity budget, and the durable store at
startup, then stops at `Configured` rather than `Ready`, because ADR 0024 records the concrete OS
process adapter as a reviewed deployment input this build does not install.

The two unavailable states are reported distinctly, because "no configuration was supplied" and
"configuration is valid but no reviewed process adapter is installed" are different operator
facts:

| State | Effect routes | Capability route |
| --- | --- | --- |
| `Unconfigured` | `503 process_lifecycle_unconfigured` | `503 process_lifecycle_unconfigured` |
| `Configured` | `503 process_lifecycle_adapter_absent` | `200`, `available: false`, `unavailable_reason: process_adapter_absent` |
| `Ready` | dispatched to the coordinator | `200`, `available: true` |

Capability stays readable whenever the deployment is configured, so an operator can see the exact
validated profile set and why effects are refused rather than debugging a silent refusal. `compose`
exists and is exercised by the focused tests, so installing an adapter is a composition change
rather than a rewrite. This is the same test-exercised, production-unwired seam pattern already
used for `handle_request`.

### Fence and liveness are separate decisions

The attached adapter's `LeaseDecisionPort` decides only the *identity* question: it compares all
five proof fields exactly, so a caller cannot present an instance, caller, session, lease, or epoch
it did not authenticate with.

It deliberately does **not** re-derive expiry. ADR 0024 records lease liveness for an attached
deployment as the adapter's own `lease_active` gate rather than a wall-clock deadline. A deadline
compared inside the port would be a second, weaker copy of a decision the HTTP gate already makes
with more information — and it would reject a request the gate accepted whenever the two horizons
disagreed. Liveness is therefore enforced exactly where it is known: the HTTP gate in
`service_lease.rs`, which runs on every request before dispatch. The bound lease states
`NO_LEASE_EXPIRY` explicitly instead of having a deadline invented for it.

## Compatibility

This is additive and gateway-local. No existing route, request or response body, protocol artifact
byte, game-mod contract, MCP frame, or harness behavior changes. An unconfigured deployment is
byte-identical to the previous behavior: the three paths fall through to the existing
`404 route_not_found`.

Rust consumers are unaffected: the change adds routes, configuration, and blanket forwarding impls
(`ProcessPort`, `Clock`, and `LeaseDecisionPort` for `Box<T>`, and `Clock` for `Arc<T>`) that only
make an already-public trait usable behind an erased or shared handle. The one component change is
`ProcessLifecycle::bind_attached_lease`, an additive public method; `Lease::new` stays `pub(crate)`,
so a caller still cannot construct a lease it was not granted.

Error mapping is stable and typed. Every `LifecycleError` maps to one fixed `(status, code)` pair,
and the response body is a JSON error object rather than a bare status.

## Deterministic oracle and evidence

`service_process_lifecycle_tests` drives the real HTTP entry point (`handle_request`) rather than
calling route handlers directly, so authorization, lease fencing, routing, and dispatch are
exercised as they ship. The process adapter is a deterministic synthetic port; these are
source-component tests and are not native game or OS-process evidence.

Four real defects were found by these tests and fixed:

1. The per-action allowed-field guard was inverted, so a valid `launch_new` was rejected `400`.
2. `parse_profiles` did not validate entries through the component constructors, so a zero profile
   id passed configuration and was rejected later at the coordinator.
3. Binding the attached lease with a live tick was unsafe: any expiry-checking fence port —
   including the crate's own default `evaluate_fence` — would then reject every request as
   expired. The adapter now states `NO_LEASE_EXPIRY` explicitly instead of deriving a deadline
   from the request, so the bound lease cannot expire on a timer nobody configured.
4. A SHA-256 digest could exceed `i64`, so the durable store failed to serialize and every
   submission returned `503 store_unavailable`; the 63-bit mask above is the fix.

Each of 1, 2, and 4 is pinned by a mutation probe: reverting the fix fails a named test rather
than passing silently. Defect 3 is pinned from the side that actually enforces liveness, because
`AttachedLeaseFence` deliberately never reads `expires_at`: removing the HTTP gate's
`lease_not_active` check fails
`liveness_is_decided_by_the_http_gate_not_the_lifecycle_fence`. That test is deliberately written
so it fails if either half of the split changes — if a future fence port starts trusting the
field, or if the gate stops checking.

The concrete native game executable, OS signal handling, host readiness, native disposable
launch/stop evidence (`sts2-gateway#50` AC5), and the harness-side client mapping
(`sts2-harness#101`) remain outside this source/component change and are `unverified`.
