# ADR 0015: bounded co-op peer-report synchronization

- Status: Accepted for coordinated integration; exact-head CI required before merge
- Date: 2026-09-06

The read-only MCP synchronization proposal needs a real producer. The attached gateway
already owns co-op coordination policy. Add a runtime ledger with configured string peer IDs
and report freshness, emitting explicitly coordinator-reported metadata under the separate
`coop-synchronization-v1` artifact, following protocol ADR 0013. The older numeric-ID local
prototype remains separate; it is not counted as a serialized-contract consumer.
This does not forward or authorize co-op gameplay, peer votes, or host mutations.

Configuration explicitly supplies two to four peer IDs and their local/ally roles. There is
exactly one local peer. No roster is inferred from connection reachability or a request body.
Unconfigured synchronization routes refuse service. On startup all configured peers are
missing; no fake initial synchronization is advertised. A fixed bounded report lifetime is
enforced using monotonic time; stale reports become missing before reads or updates.

`POST /v1/instances/{id}/coop/peer-report` accepts only peer ID, nonnegative safe-integer
generation, and connected status under the configured coordinator's control scope. Caller,
gateway/MCP sessions, instance, active lease, epoch, and correlation are fenced first.
Reports cannot change the roster or decrease a peer generation. They are coordinator input,
not independent peer authentication or native host evidence. This owner-local ingestion
request is not exported as a protocol-wide contract.

`GET /v1/instances/{id}/coop/synchronization` requires read scope, the same active binding,
and no body. It emits the complete pinned response with `source: gateway_peer_reports`.
Generation advances only after all fresh connected peers converge. Missing peers dominate
disagreement. Neither route opens a game connection, mutates a game, nor bypasses the
existing gameplay route checks. Release/shutdown fences both. There is no automatic retry
or background task; each request evaluates expiry under the existing serial worker.

This adds opt-in coordination routes without changing frozen or Runtime-v3 behavior. Tests
must cover actual HTTP admission, scopes, identities, malformed reports, roster limits,
expiry, monotonicity, convergence, release, canonical artifact output, and no downstream
access. An executable exchange with the actual MCP binary is required before merge.
