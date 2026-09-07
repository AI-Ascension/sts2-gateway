# G62: recovery allocation failure cleanup

Date: 2026-09-07. Evidence class: confirmed Linux gateway-component checks.
Implementation: `773da0051d0d815fee814d639bcc937494866c84`, based on
`23aa476c3337875b879efa823eb023d18294c8f1`.
Branch: `codex/gateway-allocation-cleanup-integration`.
The original unfinished author checkout was preserved without modification.
This evidence-only follow-up does not change the tested source.

## Reproduced failures and corrections

The carried-forward unfinished patch initially failed three of four new
allocation-failure regressions (test command exit 101):

- Local `lease_active = false` skipped durable revocation, leaving a valid row.
- A real SQLite `BEGIN IMMEDIATE` writer caused revocation to fail; after that
  writer released its lock, ordinary lease admission still accepted the row.
- An elapsed monotonic lease deadline still produced an allocation HTTP 200.

The corrected candidate passes all four. Failure closes local admission before
the fallible write and attempts cleanup even if local admission was already
closed. Ordinary lease checks and recovery acquire/renew/intent/dispatch gates
honor quarantine. The busy test positively verifies that the durable row remains
active, so denial cannot be attributed to a successful revoke.

The unfinished post-install fixture first failed before its intended injection
because it used a non-UUID gateway principal. Correcting the synthetic principal
and session made it reach an actual signed installation acknowledgment, then
inject a mismatched fence. The gateway returns 503, persists local revocation and
`PENDING_HOST_REVOKE`, retains the original lease identity, and blocks allocation.
This does not claim that the mismatched/unavailable host has revoked its lease.

The successful-cleanup test then failed because the unfinished patch supplied a
non-UUID correlation and a reason outside the closed host sideband vocabulary.
Cleanup now uses a fresh transport UUID and the accepted `shutdown` reason;
lease and installation identity remain unchanged. Signed synthetic revoke
acknowledgment is durably recorded as `HOST_REVOKED`. Without prior stop, a real
subsequent allocation installs a different lease ID and higher epoch. With prior
stop, allocation stays blocked. These are actual ephemeral TCP peer exchanges,
not managed-host or game evidence.

## Validation

Pinned compiler: `rustc 1.97.1 (8bab26f4f 2026-07-14)`, Linux. Each Cargo command
used an explicit target directory unique to this worktree.

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --all-targets --all-features --locked --quiet` | exit 0; 199 tests, including 118 runtime tests; repeated at implementation commit |
| `cargo build --workspace --all-targets --all-features --locked` | exit 0 |
| `cargo run --locked --package repo-policy -- --strict` | exit 0; implementation tree 205 sized files, zero warnings/errors |
| `git diff --check` | exit 0 |
| `sha256sum --check SHA256SUMS` in each inherited protocol artifact directory | all pass: poc-v1, runtime-v1, runtime-v2, runtime-v3-gameplay, coop-synchronization-v1 |

Allocation schema SHA-256 remains
`ee967a95e79fb2f157ce58d2b6d857de42b75f1f5ebfeb82dd9672e3b0f7670b`.
Root debug runtime binary SHA-256:
`5e42b85beaa7a6524c5419e87a4f852baa35fd4559d86d0b59d30dcec2a03d4e`.
This is a local build identity, not a reproducible-release or signed-artifact
claim; debug binaries can include build paths.

## Limits and review status

Independent non-author review is pending separately. No Windows-native gateway
test, live host cleanup, service installation, reboot, release activation,
remote publication, merge, or soak is established by this batch. No game,
provider, valued save, existing ACL or account configuration was changed.
An uncertain host revoke stays uncertain; it is not an acknowledgment.
See [ADR 0018](../decisions/0018-allocation-failure-admission.md) for ownership
and the fail-closed compatibility rule.

## Independent rejection and integrated repair

The evidence above records the earlier candidate, not approval of the later
implementation. Independent review reproduced three additional failures in
test-only commit `1b578573d1d50e83dab4dc8c7b4b0dc4d3097844`. Root integrated the
tests as `f81907adfc1deed38b2650b9c7468af797727a0e` and confirmed all three failed:
delayed successful cleanup left fresh allocation permanently closed; expiry
after durable installation left installed authority; an installed binding with
a noncanonical grant digest could pass allocation validation.

The repair retains exact-lease cleanup provenance across failed attempts,
cleans up post-commit activation failure, and validates the canonical grant on
ordinary, sideband, and allocation-response admission. The shared valid fixture
now uses the correct grant digest; the negative test explicitly corrupts only
its own SQLite row and checks rejection before cleanup. Test support was split
without a size-policy exception. Expiry uses a consumed test-only post-commit
fault, not a race against a sleeping SQLite lock holder.

An additional signed TCP regression confirms delayed ACKs do not reopen admission
after prior revocation, operator revocation, ordinary release, shutdown, an
existing shutdown request, or a marker belonging to another lease. It verifies
durable `HOST_REVOKED` while allocation remains closed in all six cases.

Root validation of this repair, using the integration worktree's unique local
`target` directory, passed formatting, diff checks, full workspace/all-target/
all-feature locked tests (122 runtime tests), strict policy (211 sized files,
zero warnings/errors), Clippy with warnings denied, and full workspace build.
The debug runtime SHA-256 is
`e9d982e61ac4d7ed23926d1e5dd7c46c2f699f0cab1d885bfc4fa0b70a3153c4`.
These remain Linux component/synthetic-transport checks. Independent re-review,
integrated release validation, and the native/live/service/soak axes remain
unverified by this repair.
