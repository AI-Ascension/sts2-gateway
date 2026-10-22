# Security Policy

## Supported state

There is no released gateway artifact yet. Security fixes apply to the current development line.
Future support windows will be recorded per release.

## Reporting

When this repository is hosted, use its private vulnerability-reporting form. If private reporting
is unavailable, open a minimal public issue requesting a private channel without exploit details.
Include the affected commit or version, platform, impact, safe reproduction conditions, and the
smallest sanitized proof. Do not include credentials, saves, process environments, personal paths,
private multiplayer data, or unbounded logs.

## Gateway threat boundary

The gateway is a high-authority confused-deputy boundary. Treat authentication bypass, unexpected
listener exposure, remote bind inheritance, arbitrary path/header forwarding, cross-instance route
selection, stale lease or epoch acceptance, replay after timeout, queue loss, crash recovery,
credential leakage, profile/save mixing, and shutdown races as security issues.

Co-op synchronization describes reports from the configured coordinator credential, which
can report every member of the configured roster under control scope. It does not authenticate
each remote peer or authorize any game mutation. Read responses retain that evidence-source
label. Roster IDs are bounded opaque lease-local references; expired reports become missing,
and lease release prevents further report ingestion or reads. A read credential cannot report.

Network reachability is never an authorization decision. A request must be bound to an authenticated
caller, session, instance, lease, epoch, fixed route, and bounded payload before forwarding. The
gateway terminates caller credentials and emits only explicitly approved downstream identity data.

Security tests must fail closed when their fixtures or credentials are unavailable. Use fake or
disposable processes and isolated ports. Do not expose a listener, use a real game profile/save,
contact a provider, or change deployment state during ordinary development validation.

## Save-profile allocation integrity

The configured allocation root and ancestors require exclusive deployment filesystem authority.
New allocation directories and provenance files use private permissions. Bounded regular-file
ownership reads reject links, unknown fields and foreign provenance; journal records cannot share
or change their immutable allocation binding. Creation and reconciled ownership require completed
file/directory syncs. Missing or invalid physical ownership retains the original operation as
unknown or blocked, with no replacement allocation. Filesystem operations do not claim protection
against a malicious concurrent writer with the same OS authority.

The attached seam validates the current admitted lease before provisioning. No static active-run
value, Runtime-v2 combat phase or allocation-directory name establishes game-mod save authority;
the missing authoritative integration keeps production mutations unavailable.

## Opt-in native process ownership

The ADR 0037 adapter trusts explicit operator configuration and the same-user systemd manager.
Its registry and user-data roots must be private. A retained random service name, full invocation
identity, boot identity, kernel process birth and exact approved executable image establish
ownership; an arbitrary PID, process-name scan or caller path never does. The adapter refuses
unknown user-data contents and a recreated registry cannot reuse the previous registry's marker.
Manager commands use fixed argument vectors with no shell or argument environment expansion.

An inaccessible cgroup is not empty. Stop completion requires retained, verified empty-cgroup
proof, and a lost command reply never authorizes another start. Unknown generations keep their
reservations. Processes sharing the gateway's OS authority can alter its private registry or
manager and are outside this isolation boundary. Host qualification requires disposable services
and unchanged unrelated controls; ordinary recording-controller tests do not establish it.
