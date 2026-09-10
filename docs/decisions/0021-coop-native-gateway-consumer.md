# ADR 0021: accepted native co-op gateway consumer

- Status: Accepted for source and serialized component integration
- Date: 2026-09-10
- Owners: gateway boundary and protocol owners

## Requirement

The gateway must expose a fixed, authenticated bridge for the accepted `coop-native-v1` envelope
without becoming a native game or multiplayer authority. Requests and responses must retain the
instance, session, lease, epoch, and correlation identity supplied by the gateway caller. A
malformed, stale, foreign, or route-incompatible message must fail closed before it leaves the
gateway.

## Decision

Consume the copied protocol artifact at schema digest
`2f3bc99e53080fa11b39592b64fb0ab964a16f568719a2622d0b2caf766ab629`. Add these exact external
routes:

| External route | Scope | Downstream route |
| --- | --- | --- |
| `GET /v1/instances/{instance_id}/coop/native/observation` | read | `GET /api/v1/coop/native/observation` |
| `POST /v1/instances/{instance_id}/coop/native/legal-catalog` | read | `POST /api/v1/coop/native/legal-catalog` |
| `POST /v1/instances/{instance_id}/coop/native/action` | mutate | `POST /api/v1/coop/native/action` |
| `POST /v1/instances/{instance_id}/coop/native/vote` | mutate | `POST /api/v1/coop/native/vote` |
| `POST /v1/instances/{instance_id}/coop/native/rejoin` | control | `POST /api/v1/coop/native/rejoin` |
| `POST /v1/instances/{instance_id}/coop/native/recover` | control | `POST /api/v1/coop/native/recover` |

The route parser is exact. The forwarder uses duplicate-rejecting JSON parsing and the copied
JSON Schema, then checks repeated headers, route kind, operation identity, catalog/effect/receipt
relations, and settled generation lineage. Observation and legal-catalog responses are read-only;
local actions and votes require an effect response; rejoin and recovery require a recovery response.
The managed producer's bounded one-field `400`, `409`, or `503` error is preserved as an explicit
host decision. Other malformed downstream bytes return `502` and never become a protocol result.

## Compatibility and oracle

This is a minor additive gateway API change. Existing routes and profiles are unchanged. The
deterministic oracle is the checked-in seventeen-golden artifact plus tests for all six route
methods, fixed downstream paths, duplicate/unknown/identity rejection, settled effect lineage,
recovery kind, lease fencing, and bounded producer errors. The artifact's `live_status` remains
`unverified`: no source/component check establishes a native host session, peer convergence,
model-controlled settlement, or disconnect/rejoin on STS2.
