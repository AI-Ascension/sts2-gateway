# Third-party notices

The gateway foundation contains no copied, vendored, or transliterated game, gateway, MCP, or
provider implementation source. Proprietary host assemblies, saves, credentials, and generated
product artifacts are not part of this target.

The target-local governance tool uses the Rust `toml` crate, pinned in the workspace manifest and
lockfile. Its upstream license and dependency metadata are consumed from the Cargo package
registry; the project does not redistribute its source. Future product dependencies require a
reviewed license, provenance, feature, and notice entry before release.

The attached Runtime-v3 adapter uses `jsonschema` 0.52.1 (MIT), pinned in the package
manifest and lockfile, with default features disabled: HTTP and filesystem reference retrieval
are not enabled. It validates the locally embedded neutral schema from `sts2-protocol`; it does
not load sibling implementation source. Cargo's locked dependency metadata records transitive
versions and licensing; no packaged release has been produced.

STS2, Slay the Spire, Slay the Spire 2, Mega Crit, Valve, and related marks or assets remain the
property of their respective owners. This independent project is not affiliated with or endorsed
by those owners and grants no rights to game files or marks.
