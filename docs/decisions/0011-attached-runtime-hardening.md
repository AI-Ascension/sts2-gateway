# ADR 0011: harden the existing attached runtime

- Status: Accepted for source/component validation; real host validation remains separate
- Date: 2026-09-04

## Requirement and owner

This decision records the independently merged main-only hardening baseline (PR #9), before
Runtime-v2 component wiring. On the component-wiring branch, ADRs 0007–0010 extend that baseline
with the optional journal, FIFO admission, scoped credentials and MCP-session fence; fixed v2
forwarding is configured there. The statements below about absent adapters/queues describe the
baseline scope, not removal of those later-composed component features.

Gateway owns bounded transport, configured endpoint trust, lease admission and retained-operation
recovery. An unauthenticated partial request must not hold the sequential service indefinitely;
slow traffic must not reset an exchange deadline. Plaintext bearer credentials must remain on
literal loopback endpoints. Releasing a fixed attached lease must not reactivate old proofs through
another allocation in that process. Exact authenticated action replay is not a new mutation.

## Decision

Use absolute five-second deadlines for inbound HTTP reads and outbound replies, and one shared
five-second downstream connect/write/read deadline. Count the header terminator within the header
limit and reject ambiguous Content-Length or Transfer-Encoding framing. Preserve the existing
request/response bounds. Configured listener and downstream addresses must be numeric loopback
SocketAddr values with nonzero ports; there is no DNS lookup or fallback endpoint.

Release permanently marks that configured lease context revoked in the running service. Further
allocation receives `lease_context_revoked`. Exact Runtime-v2 receipt replay precedes current
generation admission after identity/epoch validation. Accepted and Unknown operations may poll
read-only receipts. A retained historical settlement must not replace a newer binding observation.

Remove the policy's `bin` directory-name exclusion, split the attached implementation by owned
concern under unchanged budgets, and test actual-policy collection of executable source. No schema
artifact, new gameplay route, journal, queue architecture, V3 profile, or process supervisor is added.
No contract/implementation from an unmerged protocol proposal is required by this correction.

## Compatibility, rejection and cancellation

This is security hardening of an unpublished adapter. Non-loopback/DNS/zero-port config, stalled
traffic, ambiguous framing and released-context reuse now fail closed. Callers must migrate those
usages; a timeout or lost action response remains uncertain and cannot cause automatic mutation
retry. Release does not undo an already applied effect. Existing v2 artifacts and wire fields stay
unchanged, and the attached v2 forwarding seam remains explicitly unconfigured.

The service still has a boolean lease rather than TTL/renewal and does not persist boot epochs or
revocation. New-process admission with reused configuration remains an unresolved coordinated
architecture issue. These fixes do not prove a safe game restart or reliable autonomous run.

## Deterministic oracle

Owned synthetic sockets cover silent/slow-drip header and body deadlines, blocked writes,
exact/oversized headers and ambiguous framing. Pure admission tests reject non-loopback configuration
and same-process lease resurrection. V2 fake receipts prove exact replay causes one mutation,
Accepted reaches Settled without redispatch, payload conflicts remain rejected, identity/epoch
fences hold, and late historical results do not rewind current generation. Policy must collect
the actual runtime source and all size/license/language checks must pass without exemptions.

No game, provider, profile, save, deployment, OS-process restart or real host is used by these tests.
