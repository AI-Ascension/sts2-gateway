# `sts2-continuation-owner-adopt-v1`

This gateway-local contract adds one read-only operation to the continuation
owner routes without changing the published continuation-owner-v1 schema.

| Method and path | Recovery capability | Purpose |
| --- | --- | --- |
| `POST /v1/recovery/continuation/owner/adopt` | `continuation_owner_adopt` | Revalidate a retained operation claim and return its still-live allocation recovery authority. |

The configured recovery bearer and control scope, exact capability header,
closed frame, configured harness principal, and exact owner session are
required. Frames are limited to 16 KiB and reject duplicate JSON keys.

Requests contain the stable claim `operation_id` and exact `expected_owner`.
Success returns the matching historical claim, the same owner tuple, and the
existing `watchdog-runtime-allocation-v1` recovery authority. The authority is
reconstructed from durable authority/fence rows and includes the fence
creation timestamp and lease ID/epoch; no lease token or host proof is
returned.

Adoption begins only after current live process readiness is verified and
rechecks the owner, claim, and fence in one immediate read transaction. It
does not allocate, renew, change expiry, contact the host, or mutate the
recovery store. Repeating the same operation and owner is safe. Stale or
foreign claims conflict; expired ownership, absent ownership, revoked
ownership, and uncertain process state keep distinct refusal outcomes.

Gateway route/component tests establish this source boundary only. They do not
establish Harness resume integration, native continuation, or exact restore.
