# Repository layout

The target keeps a standard Rust governance root and one non-empty target-owned control-plane
package. Concrete adapters and product integration remain deferred.

```text
sts2-gateway/
├── .github/                 # bounded, least-privilege automation and review template
├── crates/gateway/          # target-owned lifecycle, identity, ports, and control-plane core
├── schemas/gateway/         # reserved for accepted gateway-owned shapes, not shared game rules
├── conformance/             # reserved for implementation-neutral gateway cases
├── docs/                    # architecture, policy, compatibility, testing, and decisions
├── tests/                   # reserved for owned component/conformance tests
├── tools/repo-policy/       # executable Rust governance checker
├── Cargo.toml               # locked workspace containing the gateway package and policy tool
├── Cargo.lock               # pinned governance dependency graph
└── policy.toml              # target-relative policy declaration
```

The `schemas`, `conformance`, and root `tests` directories remain reserved and do not imply a
listener, accepted wire contract, or product behavior. Do not add an empty placeholder crate to
make a command pass. Every future module needs one responsibility, a named consumer, a build/test
purpose, and an explicit boundary. The gateway package deliberately contains no concrete process,
transport, host, provider, storage, or protocol implementation.

The target does not import the sibling game-mod, MCP, or harness implementation. A future compile
dependency on `sts2-protocol` is allowed only for an accepted language-neutral and transport-neutral
contract; runtime communication remains the separate
`harness -> MCP server -> gateway -> isolated game-mod -> host` path.
