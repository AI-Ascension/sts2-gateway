# Development workflows

## Change workflow

1. Read [AGENTS.md](../AGENTS.md), the applicable decision, and the target contract documents.
2. Identify ownership, consumers, compatibility impact, security/data impact, and deterministic
   oracle before adding a public route, field, state, error, identity, or timing rule.
3. Make the smallest cohesive change and preserve unrelated files.
4. Run policy, format, Clippy, and tests; add focused deterministic tests for behavior changes.
5. Update compatibility, security, architecture, release, and changelog documents as applicable.
6. Report exact commands, results, skipped checks, evidence level, and next owner action.

## CI workflow

Pull requests and pushes to `main` run the bounded Rust quality workflow and repository policy
workflow. Both use only `contents: read`, disable checkout credential persistence, pin external
actions to immutable commits, and have explicit timeouts. Pull requests use `pull_request`, never
`pull_request_target`; no workflow may hide failure with `continue-on-error: true` or `|| true`.

The policy workflow tests and runs the target-local `repo-policy` package. The quality workflow
runs the pinned formatting, lint, and test commands. Neither workflow launches a game, provider,
deployment, or remote process. Rust adapter tests use ephemeral loopback listeners with synthetic
peers; they do not start an operator-configured service.

## Runtime and release workflow

Future runtime tests proceed from deterministic fake component tests to local component/conformance
tests, then an explicitly authorized disposable one-instance host test, and only later a controlled
multi-instance test. Runtime claims require sanitized evidence and do not arise from static checks.

Release preparation and publication are separate operations. Follow [RELEASING.md](../RELEASING.md);
no contributor or agent may publish, deploy, create tags, or rewrite artifacts without explicit
authorization.
