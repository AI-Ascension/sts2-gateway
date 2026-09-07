// SPDX-License-Identifier: MIT

# Watchdog recovery review evidence

Date: 2026-09-06

Reviewed commit: `ad842db5cb1fdfed5e8163c4cf4bd139d2c21eed`

Review worktree: `/home/timot/ascension-watchdog-work/20260906/gateway-review`

Regression worktree: `/home/timot/ascension-watchdog-work/20260906/gateway-regressions`

This packet is evidence for the gateway recovery boundary only. It does not
prove a live host, game mod, deployment, provider, or end-to-end gameplay run.
The regression tests are deliberately red at the reviewed baseline: each one
asserts the required safe result and must remain active until the corresponding
production behavior is repaired.

## Evidence classes

`RED-API` means a deterministic integration test invokes the public
`GatewayRecoveryStore` API and currently fails because the unsafe result is
accepted. `SOURCE` means a line-anchored static review of the gateway source.
`BASELINE-TEST` means an existing test or fixture demonstrates current
acceptance, but does not establish production readiness. `CONTRACT` records the
required behavior from ADR 0016 or the recovery implementation contract.

## Reproduction commands

Run from the regression worktree with an isolated target directory:

```text
export CARGO_TARGET_DIR=/home/timot/ascension-watchdog-work/20260906/gateway-regressions-target
cargo test --locked -p sts2-gateway --test recovery_safety_regressions -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::revoked_lease_cannot_record_intent --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::terminal_operation_invalidates_admission_ticket --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::witness_host_fence_identity_must_match_current_fence --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::unwitnessed_reconciled_outcome_is_rejected --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::symlink_alias_cannot_bypass_recovery_lock --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::failed_restore_does_not_leave_a_usable_clone --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::restore_generation_must_not_roll_back_live_authority --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::lease_expiry_must_not_regress_when_audit_clock_moves_backwards --exact -- --nocapture
cargo test --locked -p sts2-gateway --test recovery_safety_regressions regressions::configured_lease_ttl_and_renewal_policy_must_be_enforced --exact -- --nocapture
```

Observed Linux baseline for the aggregate regression command: **9 tests,
0 passed, 9 failed, 0 ignored**. The symlink test is Unix-only; a Windows
run has eight red tests unless an equivalent reparse-point fixture is added.

Observed locked workspace baseline after adding this packet: **117 tests,
108 passed, 9 failed, 0 ignored**. The 9 failures are exactly the tests in
`recovery_safety_regressions`; the existing 10 repo-policy, 2 library,
69 runtime, 11 control-plane, 6 co-op, 1 POC, 2 process-supervisor, and
7 recovery tests all passed.

The existing bridge fixture can be inspected with:

```text
cargo test --locked -p sts2-gateway --bin sts2-gateway-runtime host_fence_control_route_bridges_without_a_gameplay_lease -- --nocapture
```

The source anchors below were checked with `nl -ba` and `rg -n` at the reviewed
commit. No service process, listener, host, game, provider, or external system
was started by this packet.

## Findings

### R01 — persistent recovery state is not wired into the runtime

Severity: **critical**

Evidence: `SOURCE`.

Anchors: `crates/gateway/src/bin/runtime_support/service.rs:42-68,98-152`
contains in-memory `lease_active`/`lease_revoked` state and constructs the
runtime ledgers without a `GatewayRecoveryStore`. `service_lease.rs:5-31,79-105`
only flips booleans and compares static headers. `service_v3.rs:8-62` checks
those headers and forwards directly. `service_recovery.rs:6-20` only constructs
the host-fence forwarder. The durable store is exported by `src/lib.rs:36-47`,
but is not consumed by the runtime service.

Reproduction command: `rg -n "GatewayRecoveryStore|record_intent|issue_admission_ticket|reconcile_operation" crates/gateway/src/bin/runtime_support crates/gateway/src/lib.rs`.

Observed result: no runtime path opens the store or records boot, lease,
operation, ticket, or reconciliation state. The runtime can therefore report
in-memory admission while the durable recovery authority is absent.

Required safe outcome: runtime startup must open and validate the durable store;
every mutation must be recorded, fenced, and reconciled through that authority.
This source finding is not a claim that a live runtime was exercised.

### R02 — revoked leases can still record intents

Severity: **high**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store.rs:240-276` checks token,
identity, expiry, and current authority but never checks the loaded lease's
status. `recovery_store_operations.rs:17-34` calls `ensure_context` before
inserting an intent.

Reproduction: `... recovery_safety_regressions regressions::revoked_lease_cannot_record_intent --exact ...`.

Observed result: after `revoke_lease`, `record_intent` returned
`Ok(Created(...))` instead of `Err(LeaseRevoked)`.

Required safe outcome: every mutation context check must reject `REVOKED` and
`EXPIRED` leases before any operation row is created or changed.

### R03 — restore can roll authority generation back

Severity: **high**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_backup.rs:46-94` copies the backup
and calls rekey; `:100-126` derives the new generation solely from the restored
authority's generation with `current.authority_generation + 1`. There is no
monotonic external anchor.

Reproduction: `... recovery_safety_regressions regressions::restore_generation_must_not_roll_back_live_authority --exact ...`.

Observed result: a generation-1 backup was made, the live source advanced to
generation 5, and restoring the old backup produced generation 2.

Required safe outcome: restore must reject a stale namespace or establish a
generation strictly above the currently authoritative generation.

### R04 — failed restore leaves a usable un-rekeyed clone

Severity: **high**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_backup.rs:78-93` creates and copies
the destination, opens it, and then propagates `rekey_namespace` errors without
removing or quarantining the destination. Release validation at `:111-120`
returns `ReleaseMismatch` after the file has been exposed.

Reproduction: `... recovery_safety_regressions regressions::failed_restore_does_not_leave_a_usable_clone --exact ...`.

Observed result: a valid backup restored with a mismatching release returned
`ReleaseMismatch`, but the destination remained openable as a `Ready` clone.

Required safe outcome: failed restore must remove/quarantine the destination or
keep it unusable until rekey and integrity checks commit atomically.

### R05 — terminal operations do not invalidate admission tickets

Severity: **high**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_tickets.rs:133-169` validates a
ticket using only ticket state, deadline, lease, and fence; it accepts
`Issued`, `Admitted`, `Executing`, and `EffectWitnessRecorded`. It does not load
the linked operation. `recovery_store_operations.rs:133-183` settles an
operation without updating its ticket.

Reproduction: `... recovery_safety_regressions regressions::terminal_operation_invalidates_admission_ticket --exact ...`.

Observed result: after a valid `Settled` operation outcome,
`validate_admission_ticket` returned `Ok` for the still-`Issued` ticket.

Required safe outcome: ticket validation must consult the linked operation and
reject terminal/effect-witnessed operations, or atomically transition the
ticket to a terminal non-admissible state.

### R06 — `Reconciled` can be recorded without a witness

Severity: **medium**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_operation_helpers.rs:80-121`
allows unresolved-to-`Reconciled`; `:123-162` requires a witness only for
`Settled`; `recovery_store_operations.rs:154-183` then accepts direct
`record_outcome(... Reconciled, None, ...)`. The helper at `:203-220` silently
returns the unresolved operation when called without a witness.

Reproduction: `... recovery_safety_regressions regressions::unwitnessed_reconciled_outcome_is_rejected --exact ...`.

Observed result: `Unknown -> Reconciled` returned `Ok` with
`witness_present=false`.

Required safe outcome: reconciliation must require an authoritative effect
witness, or use a separately named quarantine state that cannot mean settled.

### R07 — witness host-fence identity is not checked

Severity: **medium**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_operation_helpers.rs:165-180`
checks payload digest, boot ID, and instance incarnation but omits
`host_fence_id` and the current fence. `recovery_store_operations.rs:164-170`
calls that helper before persisting the outcome.

Reproduction: `... recovery_safety_regressions regressions::witness_host_fence_identity_must_match_current_fence --exact ...`.

Observed result: a settled witness with a valid but unrelated host-fence UUID
returned `Ok(Settled)`.

Required safe outcome: compare the witness fence with the current authoritative
fence and the operation's fence context before settling or reconciling.

### R08 — lease expiry can regress under clock rollback

Severity: **medium**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_store_lease.rs:45-66` computes
`expires_at = now_millis + ttl` without comparing it to the previous expiry.
Boot, archive, and purge timestamps likewise accept caller wall-clock values.

Reproduction: `... recovery_safety_regressions regressions::lease_expiry_must_not_regress_when_audit_clock_moves_backwards --exact ...`.

Observed result: renewal at 2,000 produced expiry 32,000, then renewal at
1,000 produced expiry 31,000.

Required safe outcome: reject a backwards audit timestamp or preserve a
monotonic lease deadline using a monotonic authority clock.

### R09 — configured TTL and renewal policy are ignored

Severity: **medium**

Evidence: `RED-API`.

Anchors: `crates/gateway/src/recovery_types_ticket.rs:62-94` defines and
validates `RecoveryStoreConfig` policy fields. `recovery_store_authority.rs:185-207`
validates only the request's TTL/renewal values, and `:257-278` stores those
request values instead of applying the configured policy.

Reproduction: `... recovery_safety_regressions regressions::configured_lease_ttl_and_renewal_policy_must_be_enforced --exact ...`.

Observed result: a store configured for TTL 5/renewal 1 issued a request for
TTL 300/renewal 299.

Required safe outcome: configuration must cap or define the accepted request
policy; an out-of-policy request must be rejected or issued with bounded values.

### R10 — symlink aliases bypass the recovery lock

Severity: **high/medium**

Evidence: `RED-API` on Unix.

Anchors: `crates/gateway/src/recovery_store.rs:87-95` derives the lock path
from the caller-supplied path. `recovery_store_support.rs:14-25` opens paths
normally and follows symlinks; it does not reject aliases or enforce a secure
canonical root.

Reproduction: `... recovery_safety_regressions regressions::symlink_alias_cannot_bypass_recovery_lock --exact ...`.

Observed result: while the canonical store held its lock,
`GatewayRecoveryStore::open` through a symlink succeeded and acquired a second
alias-specific lock.

Required safe outcome: reject symlink/reparse paths, canonicalize within a
secure owner directory, and derive one lock identity for one database.

### R11 — host-fence frame validation is shallow

Severity: **medium/high**

Evidence: `SOURCE` plus `BASELINE-TEST`.

Anchors: `crates/gateway/src/bin/runtime_support/recovery_control.rs:91-145`
checks nine top-level members and only shallow string/object shape. It does not
validate UUID v4 message/correlation IDs, RFC timestamp syntax, actor/auth UUIDs,
proof bounds, or a closed complete boot object. The existing fixture at
`recovery_control_tests.rs:10-15` and the service fixture at
`service_tests.rs:208-214` use values such as `message-1`, `principal-1`,
`proof-1`, and an incomplete boot object while the bridge test expects success.

Reproduction command: `cargo test --locked -p sts2-gateway --bin sts2-gateway-runtime host_fence_control_route_bridges_without_a_gameplay_lease -- --nocapture`.

Observed result: the existing bridge path accepts the shallow fixture and
forwards it. This is not evidence that a host consumer accepted or applied it.

Required safe outcome: validate the complete closed recovery frame and all
identity, timestamp, digest, proof, and nested boot constraints before write.

### R12 — malformed post-write replies map to definite 502

Severity: **high**

Evidence: `SOURCE` and `CONTRACT`.

Anchors: `recovery_control.rs:77-87` writes before reading; `:151-158` maps
malformed/oversized reads to `MalformedResponse`. `service_recovery.rs:23-39`
maps that fault to HTTP 502, while only disconnect/timeout become the unknown
outcome. ADR 0016 lines 33-36 requires every after-write failure to be
reported as unknown and reconciled.

Reproduction command: `rg -n "MalformedResponse|MalformedResponse|outcome_unknown|map_read_error" crates/gateway/src/bin/runtime_support docs/decisions/0016-recovery-host-fence-bridge.md`.

Observed result: a mutation that may have applied before a malformed reply can
be retried as if it definitively failed.

Required safe outcome: all post-write read failures, including malformed and
oversized responses, must become unknown and enter durable reconciliation.

### R13 — bridge has no durable pending/dedup/reconciliation path

Severity: **medium/high**

Evidence: `SOURCE`.

Anchors: `service_recovery.rs:6-20` creates a forwarder per request and returns
its response; it does not open or update `GatewayRecoveryStore`. The forwarder
at `recovery_control.rs:43-88` is single-shot but has no operation record or
duplicate lookup. `service_routes.rs:22-26` exposes the route without a
recovery-store boundary.

Reproduction command: `rg -n "GatewayRecoveryStore|record_intent|lookup_operation|reconcile_operation|dedup|duplicate" crates/gateway/src/bin/runtime_support/service_recovery.rs crates/gateway/src/bin/runtime_support/recovery_control.rs crates/gateway/src/bin/runtime_support/service_routes.rs`.

Observed result: repeated identical frames are forwarded again, and an
after-write unknown has no recovery-only lookup route in the runtime adapter.
The host must independently be idempotent, but that is not a substitute for a
gateway durable record.

Required safe outcome: persist operation identity/digest before write, return
the prior receipt for an exact duplicate, reject conflicts, and reconcile an
unknown result without blindly forwarding again.

### R14 — backup/restore parent directories are not fsynced

Severity: **medium/low**

Evidence: `SOURCE`.

Anchors: `crates/gateway/src/recovery_store_backup.rs:18-40,43-94` fsyncs the
created database file but never syncs the parent directory after creating the
backup or restored destination entry. `:78-91` has the same gap for restore.

Reproduction command: `rg -n "sync_all|ensure_backup_parent|create_private" crates/gateway/src/recovery_store_backup.rs`.

Observed result: the method can report a durable-looking new path while the
directory entry itself is still vulnerable to loss on power failure.

Required safe outcome: fsync the parent directory after atomic creation/rename,
or document and enforce an equivalent durable publication protocol.

## Scope exclusions

The runtime binary and its bridge helpers are private modules, so the new
integration test intentionally exercises only the public recovery-store API.
Existing binary unit tests cover fixed-route forwarding, but this packet does
not launch a service or create a live host/mod connection. No production source,
existing test, Cargo manifest, workflow, shared checkout, host, game, provider,
or deployment state was changed.
