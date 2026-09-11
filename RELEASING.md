# Releasing sts2-gateway

No public gateway artifact has been published. Current `main` contains the Wave 2
control-plane package and a bounded attached-runtime executable: deterministic instance identity,
lease and epoch fencing, fixed route forwarding, and source-validated Runtime-v1, Runtime-v2,
Runtime-v3, and Runtime-v4 seams. The executable is intentionally fixed to one configured attached
instance; it is not a general process manager or a released gateway service. Building a candidate,
publishing a release, and verifying a published artifact are separate states; each must identify the
exact approved revision and bytes.

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
results, compatibility records, sanitized artifact inspection, and release notes. The current tree
has source/component evidence for the control plane and fixed attached-runtime routes, plus dated
attached-host and Runtime-v3 campaign records for the named STS2 v0.107.1 fixtures: the [Windows
campaign and replay](https://github.com/AI-Ascension/sts2-harness/blob/main/docs/evidence/seeded-astra-campaign-20260906.md)
and [Linux campaign and replay](https://github.com/AI-Ascension/sts2-harness/blob/main/docs/evidence/linux-seeded-campaign-20260906.md)
records. Those records demonstrate settled host actions and provider-backed campaigns for their
exact revisions, hosts, and configurations; they do not constitute a gateway package or general
compatibility claim. Gateway runtime claims require disposable process fixtures first and an
authorized controlled game environment only for the host-dependent portion. A process start, open
socket, health response, or accepted request does not prove game readiness or mutation settlement.

## Validation and publication

Run the policy command and workspace gates documented in [TESTING.md](docs/TESTING.md). Package only
reviewed bytes, the applicable license/notices, and user documentation. Exclude source metadata,
`target/`, credentials, saves, host assemblies, personal paths, and runtime logs. Record SHA-256
hashes and inspect an unpacked candidate before publication. The fixed attached-runtime binary still
has no lease TTL/renewal, durable boot-epoch rotation, or restart reconciliation, and does not prove
generic process supervision or real concurrent multi-instance isolation. For a future current
release, final current-head release artifacts, full native Runtime-v4 legality/effects, native
multiplayer, general deployment, and compatibility beyond the recorded host remain unverified. The
named Runtime-v3 records above preserve the historical settled host/provider path. These open
boundaries must be closed or explicitly excluded from the release scope before a package can be
advertised.

Only an authorized maintainer may create tags, publish artifacts, or deploy. Never publish from an
unreviewed pull request, dirty checkout, arbitrary branch, or manually supplied source path.

## Failure and rollback

Do not rewrite tags or silently replace bytes. Mark a defective release, preserve sanitized evidence,
prepare a corrective release through the same gates, and keep the last known-good version as the
rollback target. Operational shutdown or lease revocation is not release-history rewriting.
