# ADR 0029: Adoption of a live continuation owner

## Status

Accepted for the gateway component contract. Adoption recovers allocation authority for an already
live destination; it does not allocate, renew, restore, or mutate a host.

## Context

A harness resuming a selected running prefix branch must keep the existing gateway process and
destination. A new allocation request conflicts with that live lease and cannot safely replace it.
Historical continuation claims alone are insufficient: the current gateway process must still hold
the exact ready boot, host fence, installed grant, active lease, and deadline.

The v1 continuation-owner frame schema is already pinned by consumers. Adding an adopt frame to that
schema would change its digest for existing read, claim, and lookup calls. Adoption therefore uses
its own additive frame contract while preserving the v1 owner contract unchanged.

## Decision

Add `POST /v1/recovery/continuation/owner/adopt`, protected by the configured recovery bearer,
control scope, the `continuation_owner_adopt` capability header, and the closed
`sts2-continuation-owner-adopt-v1` frame. The request binds one retained operation ID and the exact
expected owner snapshot to the configured harness principal and session.

The route checks the live process-held owner immediately before beginning a SQLite `BEGIN IMMEDIATE`
read transaction. In that transaction, Gateway rechecks the current durable owner and the retained
claim for the same operation ID, and returns only if both still match the request. The response
contains the unchanged claim and owner plus the existing `watchdog-runtime-allocation-v1`
`recovery_authority`, reconstructed from the durable ready authority and current fence, including
the fence creation timestamp and the lease ID/epoch. It contains no lease token or host proof.

Adoption is read-only and idempotent. It does not issue a lease, alter its expiry, renew it, contact
the host, or fall back to allocation. A repeated request with the same operation and owner returns
the same authority. A changed owner or foreign claim conflicts. Missing, expired, revoked, or
uncertain process ownership is refused before an authority response.

Gateway owns the route, authentication, live-owner checks, store transaction, and response. Harness
owns branch selection and must install the returned authority through an explicit resume path; it
must not start a new game or replay the prefix again. Native state and effects remain game-mod
responsibilities.

## Compatibility

This is an additive minor gateway surface. Existing `sts2-continuation-owner-v1` routes, schema
digest, recovery frames, and allocation behavior remain unchanged. Only clients that request the
new route need the separate adopt-v1 frame contract. Older clients retain their existing behavior
and cannot adopt a running continuation.

## Failure and cancellation behavior

Invalid, duplicate-key, unknown-field, or oversized frames reject before a store read. Missing or
wrong recovery credentials, scope, capability, or principal reject before owner lookup. A stale
owner or foreign claim returns conflict. Expired ownership returns the expiry error; missing or
revoked ownership remains distinct; unknown live-process evidence returns unavailable. Store
contention and persistence failures return bounded retry guidance without mutating lease or claim
state. Because adoption is read-only, a disconnected caller may repeat the same request; no
mutation is retried.

## Validation oracle

The runtime dispatcher tests use a real SQLite recovery store and verify successful adoption of a
previously claimed live lease, payload-identical same-operation retry, original expiry and fence
creation time, no disclosed lease token, no store mutation, and no downstream runtime connection.
Negative cases cover missing claims, claims for another owner, expired leases, missing
installed-grant cache, stale boot lineage, wrong principal/capability, malformed frames, and
oversized frames. The route implementation calls neither allocation nor host initialization. This
is gateway source/component evidence and does not establish harness integration, native
continuation, or host restore.
