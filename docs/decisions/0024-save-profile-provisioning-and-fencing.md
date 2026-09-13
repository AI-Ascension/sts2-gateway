# ADR 0024: isolated save-profile provisioning and fenced forwarding

## Status

Proposed gateway-owned component contract. The deterministic source/component implementation is
complete for this slice; game-mod contract acceptance, launch-profile integration from issue #50,
and native host verification remain external gates.

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
gateway owner, instance, operation, and contract. The adapter must inspect the server-owned root
before creation. Unknown contents, traversal, symlink escape, foreign ownership, and implicit
overwrite/adoption are blocked without a create call. A fresh identity is allocated per operation
and is never inferred from a save-profile ID.

Provisioning and selection intent are inserted before downstream work. Duplicate operation
identity replays an identical retained result; conflicting reuse is rejected. A timeout,
disconnect, malformed response, or uncertain create remains `unknown` and retains the original
identity. Reconciliation uses the operation lookup route and never blindly repeats a mutation.
Blocked and unknown outcomes include bounded operator guidance. An active run rejects mutation
routes before provisioning or forwarding.

## Ownership and compatibility

Gateway owns the route, transport, identity, lease/epoch fence, limits, intent ledger, and
reconciliation state. Game-mod owns save-slot enumeration, current/selected semantics, baseline
calculation, and all host effects. MCP and harness clients consume the fixed gateway paths but do
not become authorities. The additive route is a proposed minor compatibility surface until the
game-mod owner accepts its versioned contract; changing paths, fields, scopes, limits, or
uncertainty semantics requires a new compatibility decision.

## Deterministic oracle and evidence

The source/component tests prove unknown-content, traversal, symlink, foreign-owner, fresh
allocation, fixed route mapping, body closure, stale lease, active-run, duplicate selection,
disconnect, timeout, and same-identity reconciliation behavior with in-memory ports and a
synthetic loopback peer. No game, save, profile, provider, or native host is used. Production
filesystem persistence, launch-profile adapter wiring, game-mod readback, and cross-restart
durability remain `unverified`.
