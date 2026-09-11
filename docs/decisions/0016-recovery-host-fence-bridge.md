# ADR 0016: fixed recovery host-fence bridge

Status: accepted for the attached runtime adapter.

## Context

The additive `watchdog-recovery-v1` contract establishes a fresh gateway boot
before a gameplay lease exists. The host must accept that boot through an
authenticated fence handshake; requiring the old gameplay lease at this point
would make recovery circular. The existing v3 gameplay routes remain frozen
and cannot carry the sideband frame.

## Decision

Expose one gateway control route:

```text
POST /v1/recovery/host-fence
Content-Type: application/json
Authorization: Bearer <gateway control credential>
```

The gateway requires its control scope and validates a bounded,
duplicate-free `host_fence_request` frame with the approved recovery contract
and schema digest. It forwards the frame once to the configured loopback mod
endpoint at the same fixed path using the mod bearer credential. The forwarded
request contains only `Authorization`, `Host`, `Content-Length`, and
`Content-Type`; it deliberately omits gameplay instance/session/lease headers,
so a new boot can be fenced before lease acquisition. The frame's actor,
authentication capability, boot identity, and correlation are the recovery
sideband's source of truth.

The bridge accepts the downstream status/body for the caller and maps only
transport failures to bounded gateway errors. A write/read failure after the
request begins is reported as an unknown host-fence outcome; callers must
reconcile according to the recovery contract and must not blindly retry.
Only numeric loopback mod endpoints are accepted. No arbitrary downstream
path, command, or proxy body is exposed.

## Compatibility and ownership

This is an additive control-plane route. It does not alter the frozen
`runtime-v3-gameplay` artifact or its six gameplay routes. Gateway owns
authentication, fixed routing, loopback enforcement, and transport bounds;
the mod/host owns atomically applying the fence and validating host-side
boot/incarnation identity. The bridge alone is not evidence that a live host
completed the handshake; that requires the mod consumer and an executable
integration trace.

## Rejection oracle

Missing or under-scoped gateway credentials fail before downstream connection.
Unknown/duplicate frame members, wrong contract/schema/kind, missing recovery
metadata, invalid configuration, oversized frames, unavailable downstream,
and malformed responses produce stable bounded errors. Old gameplay lease
headers are neither required nor forwarded.
