# Gateway negotiated capabilities v1

This Gateway-owned route artifact defines the bounded response from
`GET /v1/instances/{instance_id}/negotiated-capabilities`. It is authenticated,
lease-fenced, correlation-bound discovery evidence for MCP startup.

The route obtains a missing or authority-stale cache through the existing
validated, authenticated producer capabilities exchange, so MCP startup does not
need a manual prewarm request. It reports only fixed known game-information
operations proven by that response, including the capabilities operation itself.
It does not infer producer support from compiled Gateway routes, does not expose
bearer credentials. A successful pinned lookup-binding discovery adds only its
two fixed operations and a nonsecret witness while its Gateway authority remains
current. The nullable Runtime-v3 baseline witness appears only after a current
validated state response. It identifies the configured state adapter and the
pinned profile, with only `reobserve` and `reconcile` recovery modes; it does
not certify every native operation. The producer profile is Gateway source
identity; MCP owns any mapping to MCP tool revisions.


Each offer separates boundary units. `wire_limits.max_request_bytes` and
`wire_limits.max_response_bytes` constrain the exact serialized body Gateway
sends to, and receives from, the producer. Zero request bytes explicitly means a
bodyless operation. `content_limits` constrains the semantic content MCP may
return to its client. MCP retains its own descriptor limit for serialized
`{name, arguments}`; it must enforce an offer's wire limits after mapping the
tool call to the Gateway request and response, without deriving wrapper
overhead. `limit-examples.json` records representative bodyless capabilities,
Runtime-v3 state, lookup-binding, and paged query values.
