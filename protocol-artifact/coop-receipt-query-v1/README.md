# coop-receipt-query-v1

This directory contains the proposed, transport-neutral JSON profile for asking the
authoritative co-op host for one retained action receipt. The profile is a read-only
evidence lookup. It does not enqueue an action, reconcile a host observation, create a
new witness, or authorize a mutation.

The canonical owner is sts2-protocol. The profile is revision 1,
proposed_unadmitted, and has no serialized consumers in this candidate. The named
prospective consumers are the game mod adapter, gateway, MCP server, and harness.
Their names describe a future integration boundary; the manifest's empty consumers
array is intentional. This artifact must not be treated as integrated until each
consumer has an exact digest and producer/reader conformance record.

## Identity and evidence

Every request and response repeats the immutable operation lineage:

* operation_id, action_kind, and action_fingerprint;
* the original session_id, run_id, and location;
* actor_id, authority_id, and authority_epoch;
* the original expected_host_generation and before_host_generation; and
* the sorted, distinct participant_ids set, bounded to two through four entries.

The base envelope also binds protocol_version, the exact schema_digest,
provenance, correlation, instance, session, lease, and lease epoch. IDs are opaque
bounded ASCII values at this boundary. The mod adapter resolves them to native IDs;
native packet sender identity and authentication stay with the mod transport owner.

Responses use evidence_scope: retained_receipt. accepted, settled, and
definite prequeue rejected carry a receipt whose status matches the response.
unknown and recovery_required carry no receipt. A retained response is historical
cache evidence and cannot claim a fresh host observation or promote an operation.
settled includes the post-action generation, checkpoint, state digest, and effect
identity. Native checksum IDs/values and PacketWriter fields are deliberately absent.

Semantic checks beyond JSON Schema are required: the two generation values equal the
original admission values; participant IDs remain sorted and include the actor and
original host; a settled generation is greater than the before generation; and a
rejected receipt has definite prequeue proof. A newer generation in the same
authority can be used by a mod-side host fence, but changing the authority, session,
run, location, actor, or participant set cannot adopt the retained receipt.

## Serialization

Messages are canonical UTF-8 JSON with compact separators, a fixed contract member
order, normalized set-like arrays, no duplicate keys, and no insignificant whitespace.
Consumers reject unknown profiles, kinds, members, enum values, duplicate keys,
digest/provenance mismatches, and mismatched repeated identity. The canonical top-level
member order is protocol_version, schema_digest, provenance, correlation_id, instance_id,
session_id, lease_id, lease_epoch, kind, operation_id, action_kind, action_fingerprint,
run_id, location, actor_id, authority_id, authority_epoch, expected_host_generation,
before_host_generation, participant_ids, status, evidence_scope, receipt, error_code.
Nested order is provenance (artifact, source, generator), location (act_index, room_id,
coord), coord (col, row), and receipt (status, after_host_generation, checkpoint_id,
state_digest, effect_id, effect_kind, error_code). Checked-in JSON uses compact UTF-8
bytes followed by exactly one LF; the LF is the repository terminator, not part of the
canonical wire payload. The schema's exact UTF-8 bytes are hashed as
3e3eaedb93926b26025abb09d8028491e2632896753688c1182c698fed7d3f7c.

schema.json is the release-like copy of
schemas/coop-receipt-query-v1.schema.json; conformance.json is the copy of
conformance/cases/coop-receipt-query-v1.json. SHA256SUMS covers both copies,
the manifest, conformance case, goldens, fixtures, and both schema/case source paths.
The goldens are synthetic contract vectors and do not establish a running game,
gateway, MCP session, host installation, or end-to-end replay.
The shared conformance case has 16 vectors: six valid messages, four schema-invalid
messages, and six schema-valid messages rejected by semantic identity or settlement
rules. The semantic vectors cover actor membership, participant ordering, generation
lineage, action/effect agreement, and paired request/response identity.

## Native adapter boundary

The existing mod-owned native query contract is a separate binary artifact with its
own schema digest, ada4e23f3c17eb15ffa0420c9d5c9881bb588f2912686355fcfee8fea18b6d84.
The old managed JSON envelope claims coop-native-v1 digest
afe9bf3674f3e69b0f2454ec3fb1d6265a8e83ebccd996b6b0437208531d72b5. Neither digest
is reused here. A future mod adapter may validate this neutral profile, resolve
opaque IDs, and construct the native binary query; it must retain the native digest
and exact PacketWriter/PacketReader contract inside the mod-owned boundary.
