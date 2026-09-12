# `exact-checkpoint-reference-v1` protocol artifact (consumed copy)

Gateway-consumed copy of the `sts2-protocol/exact-checkpoint-reference-v1` release-like artifact
(schema digest `028e00d06f9f2b16cb9097f47aedd057e74046a7cb2ba97362978e18029f48ab`). It carries the
closed, digest-free public reference envelope, its goldens, and gateway-local checksums.

`crates/gateway/src/exact_checkpoint_reference.rs` verifies this copy and validates an inbound
reference at the boundary, rejecting unknown members, privileged digests, unsupported versions, and
inconsistent restore flags before any route acts on it.
