# ADR 0009: Runtime-v2 credential policy

- Status: Accepted for the attached Runtime-v2 component lane; credential issuance remains external
- Date: 2026-09-02

## Context

The attached runtime already required an exact gateway bearer value and kept the downstream mod
token separate. A static bearer without expiry, scope, or a controlled rotation window is not enough
for a multi-process coordinator. Authentication also has to happen before queue admission so an
unauthorized caller cannot consume bounded work capacity.

## Decision

The gateway uses an owner-supplied opaque credential policy. The current token is
`STS2_GATEWAY_TOKEN`; it may have a Unix expiry in `STS2_GATEWAY_TOKEN_EXPIRES_AT` and a comma-
separated scope set in `STS2_GATEWAY_TOKEN_SCOPE`. The accepted scopes are `read`, `mutate`, and
`control`. The current default keeps the existing component configuration compatible, but an
authorized live profile must set an expiry and the minimum route scopes explicitly.

During rotation, `STS2_GATEWAY_TOKEN_PREVIOUS` may remain valid until
`STS2_GATEWAY_TOKEN_PREVIOUS_EXPIRES_AT`, with scopes from
`STS2_GATEWAY_TOKEN_PREVIOUS_SCOPE`. The current and previous bearer values must differ. Both
candidate comparisons use a length-independent byte accumulator; a missing or unknown value returns
HTTP `401` `unauthorized`, an expired value returns HTTP `401` `token_expired`, and an insufficient
scope returns HTTP `403` `insufficient_scope`. The previous credential is never emitted downstream.

Read routes require `read`; v1/v2 action routes require `mutate`; allocation, release, and shutdown
require `control`. The gateway token terminates at the gateway. The separately configured
`STS2_MOD_TOKEN` is emitted only by the fixed downstream forwarder and is not accepted from caller
headers. Token values, scopes, and expiry data are excluded from metrics and diagnostics.

## Compatibility and evidence

The policy is attached-adapter configuration and does not change the frozen Runtime-v2 message
artifact. Unset expiry/rotation settings preserve the existing component default but are not a
production authentication claim. Credential issuance, secure storage, audience binding, revocation
service, and downstream token rotation remain external work for the authorized supervisor/host lane.

Component oracles must cover exact matching, missing and wrong values, expired current and previous
credentials, rotation overlap, scope denial, route-specific scope selection, separate downstream
credentials, and authentication before queue admission. A live multi-seam matrix is still required
before promoting the runtime.
