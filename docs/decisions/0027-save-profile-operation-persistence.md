# ADR 0027: Durable forwarded save-profile operations

## Decision and scope

Gateway #51 requires select/create operation intent to survive gateway restart.
`SqliteUserDataRecordStore` persists isolated allocation intent only; it is not
forwarded save-slot operation persistence. Add `SqliteSaveProfileOperationStore`
behind the existing `SaveProfileRecordStore` seam, with no attached-runtime
activation, new route, shared schema, or host/save semantics.

Gateway owns storage, identity and forwarding fences; game-mod still owns
authoritative slot/baseline readback. A separately configured journal is required;
this change does not migrate the retained allocation store or the abandoned
unpublished combined-store experiment. Existing wire types gain no serialization
API. Private persistence DTOs carry version 1 and exact canonical writer bytes.
Unknown/duplicate members, unsupported versions, malformed records, mismatched SQL
keys, request/result contradictions and terminal-state rewrites fail closed.
The dedicated journal's exact two-table/constraint-index schema is verified on
open and in each operation transaction; triggers, views, extra indexes and schema
alterations reject. Owner upsert, insert and update require exactly one affected
row, followed by token or exact-record readback before commit.

## Bounded ownership and recovery

One persistent journal contains at most 256 records, each at most 160 KiB including
bounded request and response bodies. The journal accepts one instance identity;
ledger construction rejects foreign-instance rows and rows above its own capacity.
Request identity is immutable. Results pass existing response/baseline checks;
guidance must match the status. Rows retain insertion sequence in descending order,
so sequential successful selections restore the most recent selected slot.

An exclusive canonical-path lock is held for the store lifetime. A durable random
coordinator token fences reads and writes, and immediate write transactions check
the token before admitting changes. Empty, URI and in-memory path spellings are
rejected. The deployment owner must exclusively control the database directory and
preserve database/lock files; arbitrary hostile replacement of storage is outside
this component. No secret, host path or game payload is emitted by error reporting.

Intent commits precede forwarding. Restart reconstructs the existing ledger from
validated records. A possible write is reconciled by fixed read-only operation
lookup, never by a second create/select effect. An intent with no known dispatch
remains pending when lookup has no receipt. Storage errors block further effects.

## Evidence and remaining gates

`save_profile_operation_recovery` uses real temporary SQLite files and an injected
recording mod port to check lost acknowledgments, restart, same-identity replay,
authoritative baseline, stale fences, malformed storage and coordinator fencing.
This is component recovery evidence; tests must be executed before claiming pass.

The original #51 feature also requires robust physical allocation, actual approved
launch-profile binding, attached durable dependency composition and authoritative
active-run admission. The mod #78 contract milestone and recording-mod integration
remain separate; native game acceptance is not implied. Gateway #50 lifecycle and
Harness #101 mapping also remain separate. No issue closure follows from this ADR.
