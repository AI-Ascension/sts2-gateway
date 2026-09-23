# Campaign continuation contract

- Status: Proposed; coordinated consumer migration
- Date: 2026-09-06
- Owner: Gateway

## Decision

Consume protocol PR #14 revision `a81ec64d7d14bdb3079b8c7dc3c75e5c88693dfd`.
Its schema digest is `daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b`.
The complete MIT artifact, source schema and conformance companions are copied verbatim.
The revision adds `proceed`, `confirm_selection` and `cancel_selection`.
Producer and consumers migrate together; earlier digests remain rejected.

The gateway admits the three closed payloads through its embedded producer schema and forwards them once on the existing dispatch route. The authenticated instance, caller, session, lease and epoch remain required. Extra action arguments are rejected before forwarding.

## Validation and limits

The continuation request-validation regression exercises all three canonical request vectors,
validates their authenticated context, and rejects an extra choice_id before forwarding.

Workspace tests, formatting, Clippy and strict policy pass locally. These are component results.
They do not establish available native controls, live host effects or full campaign completion.
The game-mod owns those separate host evidence requirements. Merge only with the reviewed
producer revision and coordinated consumers.
