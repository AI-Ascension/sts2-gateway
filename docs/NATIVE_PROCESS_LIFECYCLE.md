# Opt-in owned process lifecycle

The existing `sts2-gateway-process-lifecycle-v1` routes can compose a gateway-owned
Linux/systemd adapter. This is an implementation and component-test boundary.
Native systemd, game-host readiness and the harness/Studio acceptance trace remain
unverified until the disposable-host gate below is completed.

## Supported deployment boundary

The adapter requires Linux with cgroup v2, an accessible systemd **user manager**,
and both manager and command-line tools at version 254 or later. The host must
support the requested user-service sandbox properties. Missing support refuses
startup or the owned launch; it never enables a substitute shell/PID adapter.
The runtime and registry directories must be canonical, private directories
(no group/other permissions). Service-manager executables and approved game
executables must be canonical regular executable files. Paths containing
whitespace, `%`, symlink components or non-UTF-8 bytes are refused.

Gateway authentication, lease/epoch fencing, its single configured instance and
the downstream mod listener retain their existing configuration. A capability
advertising `available: true` means the configured lifecycle component is present;
it does not certify gameplay readiness or native compatibility.

Without a native configuration file, the legacy configured state remains
`available: false` / `process_lifecycle_adapter_absent`. Providing a native file
without the lifecycle deployment, or providing an invalid file, fails startup.

## Configuration

The operator supplies these existing lifecycle settings plus one native-file path:

```text
STS2_PROCESS_LIFECYCLE_ENABLED=true
STS2_PROCESS_LIFECYCLE_PROFILES=7:11:12:13:14:128:5000:5000
STS2_PROCESS_LIFECYCLE_STORE=/var/lib/sts2-gateway/lifecycle/operations.sqlite
STS2_PROCESS_LIFECYCLE_MAX_PROCESSES=1
STS2_PROCESS_LIFECYCLE_MAX_RECORDS=1024
STS2_PROCESS_LIFECYCLE_NATIVE_CONFIG=/var/lib/sts2-gateway/lifecycle/native.json
```

The numeric profile format is unchanged:
`profile:install:executable:image:namespace:descendants:start_ms:stop_ms`.
The native file must bind every declared profile exactly once. Profile IDs,
user-data roots and user-data namespace IDs cannot be shared between bindings.
Writable roots must be disjoint; neither may contain another. They cannot contain
an approved executable, manager/runtime path, native configuration or registry,
or the production lifecycle, Runtime-v2 journal or recovery store.
The lifecycle store and the native registry are separate files; preserve both.

Example file structure follows. Replace the paths and SHA-256 with those of an
approved disposable executable. The zero digest is deliberately not a working
profile; this document does not approve any game binary or installation.

```json
{
  "schema_version": "sts2-gateway-systemd-process/v1",
  "registry_path": "/var/lib/sts2-gateway/lifecycle/native-generations.sqlite",
  "runtime_directory": "/run/user/1000",
  "systemctl_path": "/usr/bin/systemctl",
  "systemd_run_path": "/usr/bin/systemd-run",
  "profiles": [
    {
      "profile_id": 7,
      "install_root": "/srv/sts2-disposable",
      "executable": "/srv/sts2-disposable/approved-fixture",
      "executable_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "user_data_root": "/var/lib/sts2-gateway/profiles/disposable-7",
      "arguments": ["--user-data={user_data}"]
    }
  ]
}
```

The file is bounded to 256 KiB, each profile has at most 64 fixed arguments of
1024 bytes each, and an executable image is bounded to 512 MiB. Arguments cannot
contain NUL, newlines or `%`. `{user_data}` is the only substitution. Other text,
including `$` and shell punctuation, is passed literally. Nothing in a lifecycle
request can alter this file, choose a path or select a manager command.

The approved executable must be inside its declared install root. Its SHA-256 is
checked before dispatch and against each observed process image. The same exact
executable is required for descendants; different helper binaries and nested
delegated cgroups are unsupported. The task limit is `max_descendants + 1`;
Linux counts threads toward that limit, so choose a bounded profile budget that
also accommodates the approved executable's threads. Inspection independently
checks the number and identity of process descendants.

The user-data root must start empty or contain the matching gateway ownership
marker. The marker binds the full profile configuration, registry location and
durable registry identity. Unknown content is preserved and refused. A new
registry cannot take over an old marker. A retained generation prevents another
generation from using its namespace until verified absence is durably recorded.

## Ownership, stop and recovery

The gateway commits an operation-generation key and a fresh service name before
dispatch. It commits a separate dispatch claim before calling `systemd-run`.
The service has `Type=exec`, `ExitType=cgroup`, `RemainAfterExit=yes`,
`KillMode=control-group`, `SendSIGKILL=yes`, `Restart=no`, `Delegate=no`, and
`ProtectControlGroups=yes`. Its writable host path is the approved user-data
root; its working directory, HOME and XDG data/config/cache locations are bound
to that root. The manager's namespace support is part of native qualification.

Recovery uses the retained generation, boot, unit, full invocation identity,
cgroup and kernel process birth. It never scans or adopts an arbitrary PID and
never resends start for a retained generation. A manager-command timeout kills
and reaps only the gateway's directly owned helper; it does not imply that the
manager cancelled an asynchronous request. No timeout permits blind retry.

Stop first persists intent, verifies the owned unit, and asks the manager to
stop its complete cgroup. Force stop targets that unit with SIGKILL, then stops
the unit. Completion requires an empty owned cgroup and terminal manager state,
or a missing unit with retained empty-cgroup evidence. Permission errors,
surviving descendants, unknown images, identity drift and missing evidence do
not become success. A stopped manager may clear live fields only when the
retained cgroup independently proves empty.

Services survive a gateway process restart. Dropping the adapter neither stops
them nor discards evidence. The existing Runtime-v2 transport shutdown/release
routes do not implicitly stop an owned service; callers must complete the fenced
process-lifecycle stop before releasing the lease when cleanup is intended. Do not delete or edit the registry, marker, unit or
operation store to clear an unknown outcome. Records are bounded and never
evicted; capacity exhaustion is an explicit refusal. Host reboot, changed
profile bindings and lost storage require reconciliation and remain fail-closed.

## Disposable-host acceptance gate

Run this gate only on a disposable supported host with an approved disposable
executable and user-data root. Keep an unrelated control process and control
directory running throughout. Record the exact gateway revision, Rust toolchain,
OS/kernel, manager version, configuration digest and executable digest. Redact
tokens, paths and manager-private identity from public evidence as required.

1. Run the full local/CI component gates, including the registry fault tests.
   Start the configured gateway and confirm authenticated capabilities and
   profile IDs. A read-only or stale-lease request must cause no process effect.
2. Submit one launch operation. Inspect its retained operation and actual owned
   service, kernel identity and isolated user-data root. Resubmit the same
   operation and verify the manager still has exactly one original generation.
3. Restart the gateway while the service remains alive. Reopen both stores,
   attach through the existing lifecycle action and verify the original
   identity. A new authority epoch must fence stale lifecycle submissions.
4. Stop and restart through the gateway. Verify the old cgroup has no members,
   the new generation has a different identity, and approved descendants are
   cleaned up. Capture an interrupted command/reconciliation case without
   creating a second process for its retained generation.
5. Confirm the unrelated process identity and control-directory contents remain
   unchanged. Confirm unknown user-data content and mismatched image/profile
   bindings are refused and preserved. Stop the final owned service through
   the gateway and retain the cleanup evidence.
6. Complete the independent harness #94 / Studio acceptance trace through the
   published gateway contract. A recording manager or direct component call
   cannot substitute for that end-to-end trace or actual host readiness.

If a supported manager, disposable executable or companion harness environment
is missing, record that exact external blocker on #50. Component tests and this
recipe alone do not close the original issue.
