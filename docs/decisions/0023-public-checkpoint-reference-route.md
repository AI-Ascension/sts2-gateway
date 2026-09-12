# ADR 0023: authorized public checkpoint reference query

Accepted for gateway source/component implementation; native producer unverified.

Requirement (CK-037/053/055): connect the pinned digest-free reference validator to an
instance-selected read operation. Gateway owns HTTP admission and authority; game owns capture
and reference production. MCP is the named HTTP consumer. Additive gateway-local route, with no
shared protocol dependency change: GET `/v1/instances/{instance_id}/checkpoint-reference`.
It is bodyless, accepts no query parameters, and requires existing read credentials and the full
instance/caller/session/MCP-session/lease/epoch/correlation fence. Dispatch uses the existing
admission queue and active recovery host grant. Shutdown/revocation refuses admission.

Forward only GET `/api/checkpoint/v1/reference` to the configured game endpoint with existing
scoped downstream credentials and current admitted authority. Response limit: 8192 bytes.
Success is HTTP 200 and a closed JSON object with exactly `schema` (literal
`ascension.checkpoint_reference_response.v1`), `instance_id`, `caller_id`, `session_id`,
`lease_id`, `lease_epoch` (integer), `correlation_id`, and `reference` (the accepted
`exact-checkpoint-reference-v1` object). All authority/correlation fields match the admitted
request. Duplicate members and extra fields at either level are refused. This wrapper is a
local transport contract, not evidence of native capture support.

No producer response, connection failure, or HTTP 404/501/503 maps to 503
`checkpoint_reference_unavailable`. Other statuses and invalid/oversized envelopes map to 502
`checkpoint_reference_response_invalid`. Bodies are never reflected on failure. Request bodies
map to 400 `checkpoint_reference_body_forbidden`; ordinary existing auth/fence errors retain
existing codes. There is no mutation, journal entry, automatic retry, retained cache, hidden
payload, caller-selected filesystem path, or assurance upgrade. Disconnect cancels only delivery
of this read; it cannot imply capture or restore completion.

Deterministic oracle: invoke the production service dispatcher with a synthetic downstream;
assert exact path, forwarded authority, valid reference bytes, fail-closed identity and scope
negatives, closed bounded response validation, and explicit unavailable without fabricated
reference. Native STS2 production and restore remain unverified.
