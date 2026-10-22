# ADR 0038: durable save-profile allocation integrity

- Status: Accepted for gateway source/component implementation
- Owner: `sts2-gateway`
- Requirement: issue #51

## Problem and decision

An allocation's recorded `Created` state is useful only while its exact physical ownership can
still be established. The existing filesystem adapter follows the provenance path, reads it
without a byte limit, and returns success before syncing the new directory and record. The
allocation journal also accepts stored identities without validating their relationship to the
operation key. These gaps must be repaired before a real launch-profile binding is composed.

The filesystem adapter keeps the existing opaque `run-<identity>` layout and provenance bytes.
It creates a private directory and exclusive private provenance file, syncs the file, then the
child and parent directories, and only then acknowledges creation. A failure after directory
creation has an unknown outcome; reconciliation inspects the original identity and never creates
a replacement. Unknown existing directories and partial records are preserved and refused.

Provenance reads accept at most 4 KiB from a regular, non-symlink file. Exact identity, closed
fields, bounded gateway provenance and canonical containment are checked before trust. The
configured root and its ancestors must remain under the gateway deployment's exclusive filesystem
authority. This adapter does not claim protection against a malicious writer with that same OS
authority racing filesystem operations.

Every restored allocation record must have a valid request fence, launch binding, descriptor and
matching gateway provenance. Operation keys and allocation identities must be unique and within
the configured budget. Durable reads validate bounded rows before decoding; writes retain the
immutable request/allocation binding and verify committed bytes. Corrupt or oversized storage
fails closed instead of becoming authority. Existing valid records retain their format.

Reconciliation re-inspects even a previously created allocation. Missing ownership becomes an
unknown outcome, and foreign or invalid contents become blocked. Neither outcome allocates a new
identity or forwards a creation based on a stale physical receipt. A valid physical record is
synced again before acknowledging ownership, so a lost prior durability acknowledgement is not
promoted solely from page-cache visibility. Inspection writes no new allocation or record bytes.

The attached handler derives save-profile authority from the lease actually admitted by its
existing lease gate. A recovery-mode lease rotation must not fall back to the static startup
lease. The save-profile seam also authorizes that context before provisioning so a stale or
foreign caller cannot reach filesystem writes through an injected composition.

## Compatibility, rejection and cancellation

No route, portable identity, schema, protocol artifact or dependency changes. Valid existing
allocation journals and provenance records remain readable. Malformed, duplicate, oversized or
misbound records that previously happened to deserialize now fail closed. Existing error enums
represent persistence, containment and unknown-outcome refusals; no host paths are returned.

Caller retries and restart recovery retain the original operation/allocation identity. No
cancellation path removes an uncertain directory, recreates a missing allocation or performs a
second host mutation. Existing profile data is preserved on every refusal.

## Deterministic oracle and limits

Tests use real temporary directories, real SQLite reopen/fault cases and a recording loopback
producer. They verify bounded provenance, symlink/unknown-content preservation, private creation,
partial-write recovery, duplicate identity refusal, row/key integrity, immutable ownership and
current-lease admission before an allocation or downstream effect.

The fixed string `automation-disposable-v1` is still a proposed save-profile binding. It is not
the numeric process profile or user-data namespace accepted by ADR 0037. An allocation identity
is also not a mod-owned save-slot identity. No child-directory layout is currently accepted by
the game-mod owner as the actual host user-data context. ADR 0041 in `sts2-game-mod` still exposes
only synthetic/unavailable discovery and selection ports and no served active-run/readback route.
Production mutations therefore remain unavailable. The later companion contract must bind the
actual instance, approved process namespace, allocation identity and baseline, establish native
root ownership before allocation, and supply authoritative active-run admission and readback.
This increment does not claim that integration, native game compatibility, deployment or release.
