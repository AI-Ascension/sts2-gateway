# `sts2-continuation-owner-v1`

This gateway-local contract provides a privileged current-owner read and an
idempotent claim for one continuation operation. It is additive and does not
change `watchdog-recovery-v1` or the Runtime-v3 gameplay contract.

The fixed routes are:

| Method and path | `x-sts2-recovery-capability` and frame capability | Purpose |
| --- | --- | --- |
| `POST /v1/recovery/continuation/owner/read` | `continuation_owner_read` | Report the current owner state and non-secret fence identity. |
| `POST /v1/recovery/continuation/owner/claim` | `continuation_owner_claim` | Compare an exact current owner snapshot and durably claim its lease for one operation ID. |
| `POST /v1/recovery/continuation/owner/lookup` | `continuation_owner_lookup` | Return an operation's historical claim separately from the live owner state. |

Each request requires the configured recovery bearer, the exact
`x-sts2-recovery-capability` header matching its frame capability, a closed
bounded frame, and an actor whose principal matches the configured harness caller.
The body is capped at 16 KiB.
Frames carry no lease token or host proof. Responses expose only identities,
lease expiry, claim digest, and claim time.

`available` means the recovery store reports a ready boot, current host fence,
active unexpired lease and installed host binding, and the live gateway process
still holds the matching Ready boot lineage, acknowledged installed host grant,
lease, and deadline. `absent`, `expired`, `revoked`, and `unknown` are distinct.
`unknown` includes persisted authority that cannot be confirmed by every live
process check. A claim is rejected unless the request exactly matches an
`available` snapshot. Historical claim rows never establish liveness.

Claims are durable, unique by operation ID and destination lease epoch, and
retained up to 4096 records with fail-closed admission at capacity. An exact
retry returns the original claim; reuse with different identity conflicts.
This gateway record protects owner admission only. Harness remains the
authority for branch lineage and execution evidence; the game-mod remains the
authority for checkpoint bytes and restore effects.

`lookup` is a reconciliation read. A historical claim does not establish that
the destination remains live and does not authorize a retry. The route reports
the current owner state independently; after process restart, unavailable
in-memory lease ownership is `unknown` even when a historical claim exists.

The JSON fixtures in this directory pin representative read, claim, and
historical lookup frames. They contain only non-secret identities.
