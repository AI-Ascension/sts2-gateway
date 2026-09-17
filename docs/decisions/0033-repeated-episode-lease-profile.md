# ADR 0033: Explicit repeated-episode lease profile

The Gateway treats an attached deployment as a single episode: `release` durably
revokes the local lease context so no later allocation is admitted. That
fail-closed default is correct for a stopped or revoked deployment, but it also
prevents two *completed* episodes from running against one deployment, which is
the concrete prerequisite recorded in `sts2-gateway#67` for the watchdog
continuous-soak campaign (`ascension-watchdog#58`).

## Decision

The Gateway adds an explicitly negotiated, gateway-local repeated-episode
profile. A caller opts in on the release that completes an episode:

```text
x-sts2-episode-profile: repeated-episode-lease-v1
```

The negotiated capability is `sts2-gateway/repeated-episode-lease-v1`, scoped to
one deployment, with the descriptor schema digest
`f3a04bab61ce4898eda0fa88cb546441493e49eef19b1e5e0841a3b4ef7c4331` (the SHA-256
of the compact canonical descriptor JSON recorded in
`service_episode_profile.rs`, reproducible without trusting the code).

Negotiation is gateway-local process state. It is deliberately **not** written to
the durable boot authority or the release set: the process, boot, fence, and
release-set identities are preserved, and a fresh process starts with no
profile. The binding records the exact boot id, instance incarnation, and
authority generation that negotiated it, so a later boot cannot inherit the
completed-episode floor.

## Semantics

- **Completed-episode release.** When a profiled release completes and the host
  has confirmed the revoke, the completed epoch becomes an admission floor, the
  permanent stop flag is cleared, and the released lease identity, deadline, and
  observed recovery catalog are dropped. The next allocation of the *same* boot
  must land on a strictly higher durable epoch.
- **Legacy default.** A release without the header is byte-identical to the
  previous behavior: the response body is unchanged and the deployment stays
  permanently revoked. No profile is recorded.
- **Stop precedence.** A release may reopen admission only for a *live* episode,
  and the stop state is captured *before* the release durably revokes the lease.
  A release that arrives while a stop is already in force — an operator revoke
  whose host acknowledgment was lost, a shutdown, or an allocation-cleanup
  retry — completes the pending revoke but never clears the permanent flag and
  never arms the profile. Because the release path sets the permanent flag
  unconditionally, reading it after the revoke could not distinguish the two, so
  the reopen decision uses the entry state.
- **Every other stop path keeps the permanent flag.** Operator revoke, explicit
  revoke, shutdown, an unresolved or rejected host revoke acknowledgment, a
  stale lease fence, a wrong caller or session, and a restart before a release
  completes all refuse admission without new effects.
- **Rotated boot.** A profile bound to a boot authority that has since rotated
  does not reopen admission; it is treated as revoked rather than ignored.
- **Fail-closed reconciliation.** The durable allocator already issues
  `MAX(lease_epoch) + 1`. An independent local floor re-checks that the acquired
  epoch is above every completed episode, so a released lease cannot be
  resurrected if the durable and local views ever disagree; a disagreement
  durably revokes the acquired lease and keeps admission closed.
- **Attached adapter.** A deployment with no durable recovery store cannot issue
  a distinct lease or epoch, so repeated episodes are unsupported there and the
  release stays permanently revoking. A profiled release with no available boot
  authority fails closed with `episode_profile_boot_required` rather than
  silently degrading to the single-episode default, and an unsupported profile
  value is rejected with `episode_profile_unsupported` before any durable write.

## Known limitations

- The witness is reported only for a release that negotiated a profile on that
  request. The stored profile persists to enforce the completed-epoch floor, but
  it is never echoed onto a header-less release, so the legacy body stays
  byte-identical even after an earlier profiled episode.

## Compatibility

Additive. The new request header `x-sts2-episode-profile` is allowlisted; no
existing route, durable record, published schema, or default response byte
changes. The profiled release body adds an `episode_profile` witness only when a
profile was accepted on that release. A later incompatibility requires a new
profile version rather than an in-place change to this one.

## Evidence boundary

Two evidence classes accompany this record, and they are not interchangeable.

`crates/gateway/tests/gateway_real_process_episode.rs` spawns the built
`sts2-gateway-runtime` binary with `std::process::Command::new` and drives it
over real loopback sockets, so every decision it asserts was made by the
*served* process: the listener, the request parser, the authorization policy,
the durable store, and the signed host sideband. This is the **spawned-process**
evidence class.

The in-process tests under
`crates/gateway/src/bin/runtime_support/service_episode_*.rs` call
`RuntimeService::handle_request` inside the test process. They are fast and
exhaustive, but they exercise a library entry point, not a running deployment,
so they must not be cited as real-process evidence.

Either way, this record claims Gateway component behavior only: no native game,
provider, deployment, or 24-hour soak is claimed, and the downstream soak in
`ascension-watchdog#58` remains separate.

### Single-process lifetime limit

The profile is gateway-local process state. It is deliberately not written to
the durable boot authority or the release set, so a **restart discards it**: a
process that did not itself negotiate the profile reports no witness for a
header-less release and stays permanently revoking. The spawned-process suite
asserts that limit directly, and any operator runbook that relies on repeated
episodes must state it. A restart also rotates the boot authority and revokes
every active lease, so a restarted process cannot reuse an earlier episode's
epoch either.
