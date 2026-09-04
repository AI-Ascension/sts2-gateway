# ADR 0010: Runtime-v2 MCP-session lease fence

- Status: Accepted for the attached component adapter
- Date: 2026-09-02

## Context

The gateway already fences caller, gateway session, instance, lease, epoch,
and correlation identities. Runtime-v2 also carries an MCP transport session
in the adapter correlation/header seam. Allowlisting that header without
checking it would permit a request from another MCP session to use an
otherwise valid lease.

## Decision

The attached gateway runtime reads `STS2_MCP_SESSION_ID`, defaulting to the
independent transport identity `mcp-session-1` (gateway session defaults to `session-1`). Every lease-protected request must carry
the exact configured value in `x-mcp-session-id`, in addition to the existing
caller/session/instance/lease/epoch fence. A mismatch or omission returns the
existing sanitized `lease_fence_rejected` response before the mod forwarder
is called.

The header remains gateway-to-MCP boundary metadata. It is not inserted into
the frozen Runtime-v2 JSON envelope and is not forwarded to the game-mod.
Allocation remains keyed by the existing control-plane identity; the harness
propagates the MCP-session value on allocation metadata and on lease release.

## Compatibility and limits

The MCP transport and gateway session use independent namespaces. The MCP default matches
the MCP process and harness defaults. An explicit override requires the same value in all three
components; deployments relying on the former inherited gateway-session default must configure
their intended MCP identity explicitly. This is a component adapter rule, not an external
issuer, revocation system, production supervisor, or host isolation proof.
The existing Runtime-v1 routes receive the same lease fence when used by the
attached runtime process.

## Evidence

The deterministic gateway service test rejects a wrong MCP session before
downstream forwarding. Full target gates pass. Live identity issuance,
rotation across all seams, multi-instance process supervision, and host
settlement remain unverified.
