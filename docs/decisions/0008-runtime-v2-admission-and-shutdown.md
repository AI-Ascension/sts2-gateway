# ADR 0008: Runtime-v2 bounded admission and shutdown

- Status: Accepted for the attached Runtime-v2 component lane; supervisor and host lifecycle remain unverified
- Date: 2026-09-02

## Context

The attached Runtime-v2 process previously handled accepted TCP connections in its listener thread.
Its operation-retention bound protected the ledger but did not bound requests waiting behind a slow
downstream mutation. A caller could therefore consume an unbounded amount of connection and request
work before the ledger rejected a later operation. The process also had no owner-controlled way to
close admission and resolve requests that had not reached the worker.

## Decision

The attached runtime uses one owner-created worker and a bounded FIFO channel. The listener parses
and bounds one request, validates the allowlisted headers and gateway bearer, then attempts a
non-blocking enqueue. The queue capacity is configured by
`STS2_RUNTIME_V2_QUEUE_CAPACITY`, bounded from 1 through 64, and queue overflow returns HTTP `429`
with `runtime_v2_queue_capacity`. Overflow never contacts the downstream mod. FIFO ordering gives
arrival-order fairness within one configured instance; this is not a global scheduler.

The worker remains the sole owner of the mutable ledger and performs all downstream I/O. It has a
bounded read and write deadline for caller connections. `POST
/v2/instances/{instance_id}/shutdown` requires the active lease and full identity fence, returns
HTTP `202`, closes lease admission, cancels queued requests with
`runtime_v2_shutdown_admission_closed`, and exits after waking and joining the listener. The active
request is allowed to finish; no admitted mutation is interrupted or retried.

`GET /v2/instances/{instance_id}/metrics` requires gateway authentication but not an active lease.
It returns only sanitized counters for seen, admitted, active, queued, completed, rejected, and
shutdown-cancelled requests, plus queue capacity and shutdown state. It contains no token, payload,
save, or private path.

The operation-retention capacity remains separate from queue capacity. A retained operation bound is
not a substitute for queue admission, fairness, cancellation, or global multi-instance capacity.
Each process still owns exactly one configured instance; a future supervisor must provide global
limits, profile/artifact isolation, crash quarantine, and lease ownership.

## Compatibility and evidence

The metrics and shutdown routes and queue-capacity setting are additive attached-adapter behavior;
the frozen Runtime-v2 message artifact is unchanged. Existing v1 routes remain unchanged. This ADR
does not claim production signal handling, downstream crash recovery, process supervision, or live
host compatibility.

Deterministic and process oracles must show that authenticated requests are FIFO within the bound,
an in-flight slow request plus a full queue yields a typed 429 without a second downstream mutation,
metrics expose the rejection, shutdown returns 202, queued requests receive an explicit cancellation
response, the listener exits, and the worker joins. The live host trace remains a separate evidence
gate.
