# ADR 0037: durable, gateway-owned systemd process generations

- Status: implementation decision; native host qualification remains unverified
- Owner: `sts2-gateway`
- Requirement: [#50](https://github.com/AI-Ascension/sts2-gateway/issues/50)
- Amends: [ADR 0035](0035-attached-process-lifecycle-route-surface.md)

## Boundary and compatibility

The first concrete process adapter supports Linux, cgroup v2, and an explicitly
configured systemd user manager version 254 or later. The existing opaque launch
profile catalog and `sts2-gateway-process-lifecycle-v1` routes do not change.
No caller can provide an executable, argument, path, environment variable, unit
name, process ID, or manager command. A separate, bounded, owner-local deployment
file binds each approved numeric profile to an exact executable SHA-256, install
directory, isolated user-data directory, and fixed argument vector.

The adapter uses fixed argument vectors for `systemd-run` and `systemctl`; it never
invokes a shell or signals a numeric PID. systemd owns each service's cgroup and
the process tree. `KillMode=control-group`, `SendSIGKILL=yes`, `Restart=no`, and
bounded start/stop and task limits are mandatory. `Type=exec` establishes only
process creation. Gameplay readiness remains the separate existing readiness gate.
`RemainAfterExit=yes` retains launch evidence after a short-lived process exits.
Environment expansion is disabled for executable arguments. Runtime configuration
is trusted operator input; another process running with the same OS authority can
alter that manager or its private records and is outside this adapter's trust boundary.

The existing disabled/configured states remain the default. The production
composition enables the adapter only when the new deployment configuration and
host checks succeed. Other operating systems, missing managers, cgroup v1,
unqualified manager versions and incomplete profile bindings remain unavailable.
An invalid explicit configuration fails startup. No existing installation is
automatically opted in and no game installation is discovered.

## Durable generation and ownership

An exclusively owned SQLite registry commits the complete operation-generation
key, exact profile/configuration digest, current boot identity, and an unpredictable
unit name before any manager request. A second durable transition records that a
start may be dispatched. A retained generation is lookup-only: a lost manager reply,
gateway restart or caller retry never repeats `systemd-run` for that identity.
Unknown results retain capacity through the existing lifecycle ownership ledger.
The registry also has a durable random identity. Each isolated user-data marker
binds that identity and registry location; recreating or redirecting storage cannot
silently reuse a previously owned namespace. An unresolved generation excludes
other generations from its user-data namespace. Row keys, handles, immutable
identity fields and stage transitions are checked on access and reopen.

Successful observation binds the full systemd invocation ID, cgroup path, main PID,
kernel birth identity, executable image, instance and isolated user-data namespace.
Those private records supplement the existing portable process identity; host paths
and systemd identifiers are not added to the HTTP contract. Attach, inspect and stop
resolve only a retained registry handle and verify its unit and invocation. There
is no process adoption by PID, executable-name scan or instance/profile fallback.

Every stop persists its intent and targets only that exact owned unit. Completion
requires an observed empty owned cgroup. Timeout, identity drift, unknown image,
out-of-bound descendants or unavailable inspection remain explicit failures; none
is a stopped result. A changed boot or missing unit without sufficient retained
absence evidence is indeterminate. The existing generation-recovery seam has no
absence variant: an identity-less generation stays indeterminate even when the
registry proves it was never dispatched. A retained identity can report exited
only after child-free absence is verified. Records are bounded and never purged.

The approved install and executable must resolve without symlinks and the image
must match the configured hash. The isolated user-data root must either be empty
or carry this gateway's matching ownership marker. Unknown contents are preserved
and refused. Host paths, argv, environment, manager output and filesystem errors
are never copied into portable responses or diagnostics.
Writable roots are pairwise disjoint and cannot include approved executables,
manager paths, configuration or owner journals. This rejects configuration-level
overlap before a service is allowed to write; it is separate from the same-user
manager trust boundary above.
See [the deployment guide](../NATIVE_PROCESS_LIFECYCLE.md) for the closed file
format, path/argument bounds and the separate disposable-host acceptance gate.

## Timing and cleanup

Manager commands have explicit deadlines and bounded output. The gateway owns and
reaps command children; timeout terminates only that directly owned helper and does
not infer that an asynchronous manager request was cancelled. The durable operation
remains available for reconciliation. No helper timeout authorizes relaunch.

Every filesystem coordinator lock is explicitly unlocked after its SQLite connection
closes. A helper can briefly inherit the same open-file description before exec;
closing only the parent's descriptor would prolong ownership until that child closes
its copy. The private guard preserves immediate exclusive admission and releases the
retired owner's lock even while a duplicate descriptor remains open. Store construction
also establishes this teardown order before fallible database initialization. Unlock
failure never authorizes a competing owner; descriptor closure remains the fallback.

Service processes are owned by the manager and survive a gateway transport restart.
Dropping the adapter does not stop them or discard their records. Explicit lifecycle
stop/shutdown policy remains gateway authority; process survival is never presented
as completed cleanup. A native service may execute only with a separately configured
host and approved disposable profile.

## Deterministic oracle and evidence gates

Recording-controller tests cover the real generation registry and ProcessPort:
duplicate and reopened recovery, pre-dispatch and post-dispatch interruption,
wrong profile/image/instance/birth/invocation, retained descendants, timeout,
capacity, and exact restart generations. Fixed-argv tests prove caller values cannot
select manager operations. Existing authenticated lifecycle-route tests continue
to cover lease/epoch and authority boundaries. All workspace gates remain required.
Deterministic duplicate-descriptor tests separately prove retained-lock behavior without
the guard, immediate refusal while the owner lives, and authority release on guard drop
while a duplicate remains open. They do not depend on reproducing a thread scheduling race.

Native qualification requires an opted-in disposable systemd service, an unrelated
control process and user-data tree, exact component/configuration hashes, launch,
gateway restart/attach, stop/restart, descendant cleanup and unchanged controls.
Recording-controller and ordinary child-process tests do not qualify systemd,
game-host readiness or the harness/Studio end-to-end gate. This ADR does not close
#50 or harness #94 solely from component evidence.

## External interfaces

The implementation uses documented systemd service and kill behavior:
[systemd-run](https://www.freedesktop.org/software/systemd/man/255/systemd-run.html),
[systemd.service](https://www.freedesktop.org/software/systemd/man/256/systemd.service.html),
and [systemd.kill](https://www.freedesktop.org/software/systemd/man/254/systemd.kill.html).
These are host interfaces, not Rust dependencies or redistributed source. Existing
SQLite, SHA-256, serialization, UUID and filesystem-lock dependencies suffice.
