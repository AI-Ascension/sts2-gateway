# Releasing sts2-gateway

No gateway artifact is released from the foundation wave. Building a candidate, publishing a
release, and verifying a published artifact are separate states and require explicit maintainer
authorization.

## Version model

Keep these versions distinct:

- repository and governance-tool version;
- gateway API/control-plane version;
- fixed game-mod HTTP contract version;
- game-host and loader compatibility range;
- MCP server revision; and
- harness client contract revision.

A host update does not automatically change the gateway API. A route, identity, lease, fence,
authentication, error, timing, or isolation change requires a compatibility classification and
decision. Stable release tags and artifacts are immutable; corrections use a new version.

## Readiness

A candidate requires the exact approved commit, required review, policy/format/lint/test/conformance
results, compatibility records, sanitized artifact inspection, and release notes. Gateway runtime
claims require disposable process fixtures first and an authorized controlled game environment only
for the host-dependent portion. A process start, open socket, health response, or accepted request
does not prove game readiness or mutation settlement.

## Validation and publication

Run the policy command and workspace gates documented in [TESTING.md](docs/TESTING.md). Package only
reviewed bytes, the applicable license/notices, and user documentation. Exclude source metadata,
`target/`, credentials, saves, host assemblies, personal paths, and runtime logs. Record SHA-256
hashes and inspect an unpacked candidate before publication.

Only an authorized maintainer may create tags, publish artifacts, or deploy. Never publish from an
unreviewed pull request, dirty checkout, arbitrary branch, or manually supplied source path.

## Failure and rollback

Do not rewrite tags or silently replace bytes. Mark a defective release, preserve sanitized evidence,
prepare a corrective release through the same gates, and keep the last known-good version as the
rollback target. Operational shutdown or lease revocation is not release-history rewriting.
