# ADR 0009: Runtime-v3 framing and authenticated envelope fencing

- Status: Accepted for source/component validation; live integration remains unverified
- Date: 2026-09-04

## Requirement and ownership

The gateway must not forward a body whose instance, session, lease, epoch, correlation, or
request kind differs from the authenticated fixed route. It must not relay a response from
another context or operation. The game-mod still owns catalog admission, host observations,
operation deduplication, and independent settlement evidence; schema validity is not host proof.

## Decision

Embed the inert canonical `runtime-v3-gameplay` schema and artifact, validate with pinned
`jsonschema` 0.52.1 without network/filesystem resolution features, and apply the profile's
neutral cross-field comparisons and UTF-8 bounds locally. Accept no additional or duplicate
fields. Require `application/json` even for the profile's body-bearing GET requests. The exact
schema digest is pinned, not merely compared against another untrusted request value.

Incoming request reads and outgoing response writes each have an absolute five-second deadline.
The downstream connect/write/read exchange shares a five-second deadline. Reject framing with
transfer encodings or ambiguous lengths, and count the terminating delimiter in the header bound.
Both configured addresses must be literal loopback addresses with nonzero ports: there is no DNS
resolution or fallback host. The attached runtime remains sequential; bounded waits are not
multi-client fairness or concurrent admission.

Releasing the configured lease permanently revokes that context in the running service.
Reallocation of the same released context returns `lease_context_revoked`. This does **not**
persist revocation across a process restart. Boot-epoch rotation, historical receipt retention,
lease expiry/renewal, and graceful signal shutdown remain unresolved integration requirements;
the generic library and injected supervisor do not implement them for this executable.

## Compatibility and uncertainty

These are security corrections to the unpublished attached adapter. Clients using DNS names,
non-loopback endpoints, missing content types, malformed profile envelopes, or released-context
reuse now fail closed. They must update before using this branch. A timeout or malformed response
after dispatch leaves the operation outcome uncertain; clients retain its operation identity and
may only reconcile/read, never blindly send a new mutation. Errors expose stable codes, not bodies
or credentials. A recovery response of an unexpected message kind is rejected rather than
misrepresented as a valid catalog or observation.

## Deterministic oracle

Synthetic loopback tests exercise stalled headers/bodies/writes, slow-drip deadlines, exact and
oversized header bounds, ambiguous framing, route kinds, header/body mismatches, wrong response
identity/operation/correlation, duplicate JSON keys, unknown fields, missing required nulls,
oversized numeric/UTF-8 values, duplicate catalog IDs, and observation/witness mismatches.
No licensed process, deployment, provider, profile, or save is used.
