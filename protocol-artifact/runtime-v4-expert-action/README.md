# `runtime-v4-expert-action` additive potion action transport

This artifact binds one host-generated `use_potion` legal action to the gateway lease, session,
generation, state, correlation, and operation identities. A response is settled only when the
host returns a fresh expert observation and a `potion_use_settled` transition witness. An accepted
operation whose response is lost remains `unknown` until the same operation identity is reconciled.

The artifact is inert protocol evidence. The game mod owns host legality and mutation, the gateway
owns authentication and forwarding, MCP owns mapping, and the harness owns coordination.
