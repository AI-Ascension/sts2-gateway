# ADR 0024: isolated save-profile provisioning and fenced forwarding

## Status

Proposed gateway-owned component contract. The deterministic source/component implementation is
complete for this slice, but the attached runtime stays explicitly unprovisioned until a real
isolated-allocation port, launch-profile binding port, durable operation-intent store, and
authoritative active-run source are accepted; game-mod contract acceptance, launch-profile
integration from issue #50, and native host verification remain external gates.

## Context

Issue #51 needs automation to discover and select save profiles without giving a caller a path,
process command, URL, or authority over game state. The game-mod remains the owner of save-slot
meaning, baseline contents, and host-thread effects. The gateway owns instance/session/lease
fencing, isolated user-data allocation, operation intent, bounded forwarding, and recovery
outcomes.

The launch-profile shape from issue #50 is not merged on this branch. A gateway-local contract
constant and `LaunchProfileBindingPort` isolate that dependency. Only the fixed
`automation-disposable-v1` profile is admitted; callers cannot provide launch settings.

## Decision

The attached runtime admits only these instance-scoped paths:

| Method | Path suffix | Scope | Body |
| --- | --- | --- | --- |
| GET | `save-profiles` | read | empty |
| GET | `save-profile/current` | read | empty |
| POST | `save-profile/select` | mutate | exactly `profile_id` (or the legacy alias `save_profile_id`) and a validated `baseline` |
| POST | `save-profile/create-disposable` | mutate | empty or `{}` |
| GET | `save-profile/operations/{operation_id}` | read | empty |

Every route requires the configured caller, session, instance, active lease, lease epoch, MCP
session, correlation identity, and an authenticated scope. Bodies and operation IDs are bounded at
16 KiB and 128 bytes respectively. The gateway maps the routes to fixed loopback game-mod paths;
it never forwards a caller path, header, command, URL, profile root, or arbitrary JSON member.
List/current discovery does not require a future baseline. Selection requires a baseline before
forwarding, while disposable creation returns the authoritative user-data descriptor and baseline
from the game-mod response.

User-data allocation uses an opaque nonzero identity and a provenance record containing only the
gateway owner, instance, operation, and contract. The allocation adapter must inspect the
server-owned root before creation. Unknown contents, traversal, symlink escape, foreign ownership,
and implicit overwrite/adoption are blocked without a create call. A fresh identity is allocated per
operation and is never inferred from a save-profile ID. The launch binding is never built by the
gateway: the injected `LaunchProfileBindingPort` returns it for the reserved identity, and a
refused binding blocks the allocation entirely.

Provisioning and selection intent are inserted before downstream work. Duplicate operation
identity replays an identical retained result; conflicting reuse is rejected. A timeout,
disconnect, malformed response, or uncertain create remains `unknown` and retains the original
identity. Reconciliation uses the operation lookup route and never blindly repeats a mutation.
Blocked and unknown outcomes include bounded operator guidance. A mutation is admitted only when
an authoritative active-run source reports that no run is in progress and the operation intent is
recorded in an injected durable store; an active run, an unconfigured active-run source, and a
volatile-only intent store each reject the mutation before provisioning or forwarding. The attached
runtime composes none of those dependencies yet, so it reports an explicit unavailable capability
instead of creating production in-memory substitutes. A creation receipt must echo the reserved
identity and the approved launch contract, otherwise the response is invalid and the operation
remains unknown.

## Ownership and compatibility

Gateway owns the route, transport, identity, lease/epoch fence, limits, intent ledger, and
reconciliation state. Game-mod owns save-slot enumeration, current/selected semantics, baseline
calculation, and all host effects. MCP and harness clients consume the fixed gateway paths but do
not become authorities. The additive route is a proposed minor compatibility surface until the
game-mod owner accepts its versioned contract; changing paths, fields, scopes, limits, or
uncertainty semantics requires a new compatibility decision.

## Deterministic oracle and evidence

The source/component tests prove unknown-content, traversal, symlink, foreign-owner, fresh
allocation, fixed route mapping, body closure, stale lease, active-run, missing active-run source,
missing durable intent, duplicate selection under a distinct operation ID, disconnect, timeout,
reserved-identity receipt binding, launch-binding refusal, and same-identity reconciliation behavior
with in-memory ports and a synthetic loopback peer. Refusal cases inject the adapter's inspection
classification or typed port error rather than traversing a real root, and restart cases reopen a
shared record store rather than a real durable store file. No game, save, profile, provider, or
native host is used. Production filesystem allocation and containment, a durable store adapter,
launch-profile adapter wiring, game-mod readback, and cross-restart durability remain `unverified`.
