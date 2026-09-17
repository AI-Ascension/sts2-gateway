# `game-information-live-observation-bootstrap-v1`

This candidate artifact defines a bounded, read-only negotiated bootstrap for the first
entity-scoped live request using the accepted `game-information-query-v1` profile. It is owned by
`sts2-protocol`; the prospective producer is `sts2-game-mod`, and the named prospective consumers
are `sts2-gateway`, `sts2-mcp-server`, and `sts2-harness`.

A bootstrap response carries a trusted query-v1-shaped `parent_observation` and a bounded list of
visible entity references with their matching snapshot references and optional content-manifest
definition mappings. The selector may match multiple occurrences of one definition. Each occurrence
remains distinct by its native `instance_ref`; a consumer must choose one exact reference and its
attested snapshot before forming an unchanged query-v1 live request. Wildcard, generated,
process-local, and transport-lease identities are invalid substitutes for native references. The
scope binds the gateway instance, native run, manifest, locale, and positive harness authority epoch.
Owner labels describe responsibility and do not replace the authenticated gateway/lookup-binding
fence. Native occurrence and snapshot identities are owner-local to the observed host surface;
restart, restore, content/profile/run replacement, or epoch change invalidates them, with no
persistence claim across restart.

Owner provenance is explicit: game-mod owns the native snapshot, content manifest, and occurrence
epoch; gateway owns the authenticated instance fence; harness owns the authority epoch. The
transport lease epoch is a fence only and is never copied into the query occurrence reference.

This artifact is additive. It does not change the bytes or digest of `game-information-query-v1` or
`game-information-lookup-binding-v1`, and it defines no route, host access, credentials, lifecycle,
persistence, or mutation behavior. The candidate's schema and vectors are synthetic serialization
evidence. Native extraction, consumer adoption, gateway/MCP/harness route integration, and live
agent reachability remain unverified.
