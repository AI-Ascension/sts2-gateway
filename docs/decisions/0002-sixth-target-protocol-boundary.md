# ADR 0002: Current sixth-target protocol boundary

## Status

Accepted by the current build-completion orchestration for the foundation wave. This ADR resolves
the target-set decision; it does not add a gateway dependency or protocol implementation.

## Context

Earlier planning material treated `sts2-protocol` as a deferred decision-stage candidate. The
current build-completion instruction explicitly accepts six targets and names `sts2-protocol` as
the sixth target. The gateway still needs to avoid using a shared repository as a generic home for
boundary-specific lifecycle or routing behavior.

## Decision

`sts2-protocol` is an accepted, narrowly owned repository for genuinely shared,
language-neutral, transport-neutral contracts with at least two named consumers, explicit
versioning, provenance, and implementation-neutral conformance. The gateway may consume such a
contract only after its owner, consumers, compatibility behavior, and release path are accepted.

The gateway remains the normative owner of instance lifecycle, leases, lease epochs, fencing,
authentication, route/method/header/body policy, process ownership, readiness, health, isolation,
backpressure, and cleanup. Gateway-specific records and policies stay local. No path dependency,
generated shared type, game rule, host object, MCP tool, model record, or arbitrary route is moved
to `sts2-protocol` merely for reuse.

## Alternatives considered

1. Continue treating the protocol target as unapproved: rejected for this run because the current
   instruction supersedes the earlier disposition.
2. Put every cross-target type in the protocol target: rejected because it would erase ownership,
   increase release coupling, and make boundary-specific behavior ambiguous.
3. Add a gateway-to-protocol dependency during foundation preparation: rejected because no accepted
   neutral lifecycle contract is implemented yet.

## Consequences

The sixth target receives its own foundation and later contract review, while this gateway remains
buildable without it. A future neutral identity or lifecycle contract must identify producer,
consumer, format, uniqueness/lifetime, restart and collision behavior, security status, compatibility
version, and conformance fixture before adoption. The stale planning recommendation remains historical
context; this ADR is the target-local record of the current instruction.
