## Change

Describe the gateway boundary, requirement, or foundation rule affected. Identify any lifecycle,
lease, identity, route, authentication, isolation, or data-handling contract touched.

## Evidence

- [ ] I ran the target-local policy command.
- [ ] I ran formatting, lint, and tests applicable to this change.
- [ ] I recorded exact commands and results.
- [ ] Runtime, host, provider, and deployment claims are labelled with their evidence level.

## Boundary review

- [ ] No game rules, host objects, loader code, MCP semantics, model/provider behavior, or arbitrary
      proxying was added to the gateway.
- [ ] Every forwarded request remains fixed-route, fixed-method, bounded, and lease/fence checked.
- [ ] Queues, deadlines, cancellation, shutdown, and accepted-but-unknown work are explicit.
- [ ] No credentials, saves, proprietary files, generated output, or machine-specific paths are in
      the change.
- [ ] Documentation, compatibility, security, and changelog impact is addressed.

## Follow-up or blocker

List unresolved contract decisions, unavailable runtime checks, or the next owner action.
