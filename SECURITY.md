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

Network reachability is never an authorization decision. A request must be bound to an authenticated
caller, session, instance, lease, epoch, fixed route, and bounded payload before forwarding. The
gateway terminates caller credentials and emits only explicitly approved downstream identity data.

Security tests must fail closed when their fixtures or credentials are unavailable. Use fake or
disposable processes and isolated ports. Do not expose a listener, use a real game profile/save,
contact a provider, or change deployment state during ordinary development validation.
