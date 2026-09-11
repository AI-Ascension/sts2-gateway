# Repository layout

The target keeps a standard Rust governance root and one non-empty target-owned control-plane
package plus a separate attached runtime binary. General lifecycle adapters remain unverified.

```text
sts2-gateway/
├── .github/                 # bounded, least-privilege automation and review template
├── crates/gateway/          # target-owned lifecycle, identity, ports, and control-plane core
├── protocol-artifact/poc-v1 # offline release-like artifact copy used by the POC proof
├── schemas/poc-v1.schema.json # protocol-owned source-path mirror for artifact provenance
├── schemas/gateway/         # reserved for accepted gateway-owned shapes, not shared game rules
├── conformance/             # copied protocol companion and future gateway cases
├── docs/                    # architecture, policy, compatibility, testing, and decisions
├── tests/                   # reserved for owned component/conformance tests
├── tools/repo-policy/       # executable Rust governance checker
├── Cargo.toml               # locked workspace containing the gateway package and policy tool
├── Cargo.lock               # pinned governance dependency graph
└── policy.toml              # target-relative policy declaration
```

The `schemas/poc-v1.schema.json` file is a verbatim protocol-owned source-path mirror required by
the copied artifact's manifest and README; it is not a gateway-owned schema. The `schemas/gateway`
and root `tests` directories remain reserved and do not imply a listener, accepted wire contract, or
product behavior. `conformance/cases/poc-v1.json` is a verbatim protocol-owned companion copied only
because the normative artifact checksum inventory covers it; it does not move protocol ownership or
implementation into this repository. Do not add an empty placeholder crate to
make a command pass. Every future module needs one responsibility, a named consumer, a build/test
purpose, and an explicit boundary. The gateway package deliberately contains no concrete process,
transport, host, provider, storage, or protocol implementation. The attached runtime adapter is an
explicitly separate binary under `crates/gateway/src/bin/`.

The target does not import the sibling game-mod, MCP, or harness implementation. The POC consumes a
checked-in artifact copy and has no protocol implementation path dependency. A future compile
dependency on `sts2-protocol` is allowed only for an accepted language-neutral and transport-neutral
contract; runtime communication remains the separate
`harness -> MCP server -> gateway -> isolated game-mod -> host` path.

## Naming authority

The aggregate NAMING_CONVENTIONS.md and its naming-registry.yaml define shared
casing and identity vocabulary. Gateway-owned route or wire names require this target's compatibility
review; a concise directory or equal suffix does not create a shared identity.

## Runtime-map additions

```text
protocol-artifact/runtime-map-v1/             copied manifest, schema, goldens, and checksums
schemas/runtime-map-v1.schema.json            source-path schema companion
conformance/cases/runtime-map-v1.json         implementation-neutral consumer case
crates/gateway/src/bin/runtime_support/runtime_map*  fixed route and response validator
```

These are inert contract data and gateway-owned transport validation. The protocol owner remains
responsible for the neutral model and conformance meaning; the game-mod remains responsible for
host observation and projection.

## Runtime-v4 expert rest-action additions

```text
protocol-artifact/runtime-v4-expert-rest-action/  candidate manifest, schema, goldens, mutations, producers, checksums
schemas/runtime-v4-expert-rest-action-v1.schema.json  protocol-owned source-path mirror
conformance/cases/runtime-v4-expert-rest-action-v1.json  implementation-neutral consumer case
crates/gateway/src/bin/runtime_support/runtime_v4_expert_rest_action*  fixed route, forwarder, and semantic checks
```

The profile is a candidate protocol copy with no admitted consumers. The gateway owns only the
transport and bounded validation; rest-site rules, native producer serialization, host effects,
MCP mapping, and harness behavior remain outside this target.

## Native co-op additions

```text
protocol-artifact/coop-native-v1/           accepted manifest, schema, goldens, producer and consumer records
schemas/coop-native-v1.schema.json          verbatim protocol-owned source-path mirror
conformance/cases/coop-native-v1.json       implementation-neutral consumer case
crates/gateway/src/bin/runtime_support/coop_native*  fixed route and closed-envelope validation
crates/gateway/src/bin/runtime_support/service_coop_native.rs  lease-fenced downstream bridge
```

The gateway owns only the route, transport, identity, lease, and response relation checks. Native
peer admission, legal game actions, shared effects, checksum production, and rejoin authority stay
in the game-mod host boundary.
