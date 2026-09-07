# Gateway coding standards

## Boundary rules

Name modules for one owned concern: identity, lifecycle, lease/fence, routing, authentication,
health, process ownership, capacity, or cleanup. Keep composition roots thin. Keep transport
decoding and encoding at the edge and lifecycle decisions in testable policy code. Do not create
`utils`, `helpers`, `common`, or `manager` modules as ownership escapes.

The gateway is not allowed to contain game rules, host types, loader/ABI code, MCP framing or tool
catalogs, model/provider clients, harness artifacts, or arbitrary proxy behavior. The initialized
gateway crate depends on explicit ports and owner-local types; it must not depend on the game-mod
implementation or a concrete host.

## Rust and size

Use Rust `1.97.1`, edition 2024, `rustfmt`, and Clippy warnings denied. Keep production Rust below
300 nonblank lines and never above 400 without an exact policy exception; keep tests below 400 and
never above 600. Keep functions at or below 40 lines where practical, split beyond 60, and never
use line compression to evade the budget.

Use the smallest visibility that satisfies a consumer. Public types document behavior, errors,
panic conditions, compatibility, and security implications. Prefer explicit newtypes and enums for
identity, lifecycle, lease epochs, deadlines, route names, and bounded sizes.

## Error and concurrency rules

Use structured `Result` errors and stable boundary mappings. Distinguish malformed input,
unauthorized caller, wrong instance, expired lease, stale epoch, unavailable process, timeout,
cancellation, overload, downstream failure, and unknown outcome. Do not branch on display strings.
Do not claim a queued request is complete when the contract requires settlement.

Bound every queue, body, header set, response, log field, and cleanup operation. Use injected
monotonic time for durations and avoid arbitrary sleeps. Do not hold locks across transport calls,
process waits, callbacks, blocking I/O, or user code. Every task/thread has an owner and join path;
shutdown closes admission and resolves outstanding work.

## Safety and security

Unsafe Rust is forbidden in this target by workspace policy. Never log credentials, full payloads,
process environments, saves, private paths, or multiplayer identifiers. Validate authentication,
authorization, session, instance, lease, epoch, route, method, headers, and body before forwarding.
Network reachability is not authentication. Do not accept arbitrary paths, headers, redirects, or
downstream credentials.

## Serialization and dependencies

Externally visible fields, enum values, status codes, error codes, content types, optionality,
ordering, and bounds require explicit names and exact fixtures. Add round-trip and malformed-input
tests for every accepted contract. Keep gateway-specific contracts here; move a type to
`sts2-protocol` only under [ADR 0002](decisions/0002-sixth-target-protocol-boundary.md).

Before adding a dependency, check the standard library and existing ports, then record license,
MSRV, feature, security, and boundary impact. Pin versions through the workspace lockfile. This
initialization package adds no network, runtime, process, or serialization dependency; any future
adapter dependency needs an explicit boundary decision.

## Aggregate naming authority

Use the aggregate [`NAMING_CONVENTIONS.md`](../../planning/naming_conventions/NAMING_CONVENTIONS.md)
and [`naming-registry.yaml`](../../planning/naming_conventions/naming-registry.yaml) for casing,
identity namespaces, lifecycle vocabulary, evidence states, and protected routes or fields. Gateway
identities such as sessions, instances, operations, and lease epochs remain distinct from MCP,
host, core, and harness identities.
