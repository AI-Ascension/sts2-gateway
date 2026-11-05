# ADR 0039: finite preapproved native-root binding for disposable save profiles

- Status: **Proposed owner decision; no runtime adoption or native acceptance**
- Owner: `sts2-gateway` for native roots, process identity and durable allocation; `sts2-game-mod` for host-thread save-slot semantics
- Requirement: [Gateway #50](https://github.com/AI-Ascension/sts2-gateway/issues/50), [Gateway #51](https://github.com/AI-Ascension/sts2-gateway/issues/51), [GameMod #78](https://github.com/AI-Ascension/sts2-game-mod/issues/78)
- Extends: [ADR 0037](0037-owned-systemd-process-generations.md), [ADR 0038](0038-save-profile-allocation-integrity.md)
- Compatibility: internal design proposal only; existing process/save wire schemas, paths and default disabled behavior remain unchanged

## Working architectural decision

Select a **bounded, finite, operator-preapproved disposable native-profile root pool** for the first candidate integration. Each root remains the exact fixed `user_data_root` of one approved numeric process profile under ADR 0037. Do **not** add a caller-controlled or dynamically interpolated root, convert `UserDataIdentity` to `LaunchProfileId`, or infer GameMod `SaveSlotId` from numeric host profile IDs. The alternative dynamic-root ABI is reserved for a separately reviewed and versioned owner change.

**This is a proposal, not permission to activate the integration.** Owner acceptance requires an exact physical-root/provenance reconciliation between ADR 0037's native owner marker and ADR 0038's `run-<identity>` child layout. That mismatch currently has no approved adapter. The Gateway and GameMod owners must qualify the explicit contract and native readback before any production composition or merge. Until then `SaveProfileDependencies::unavailable()` and `SaveProfileActiveRun::UNCONFIGURED` are safety requirements, not technical debt to bypass.

## Source contract and qualification detail

**State of evidence: PROPOSED; no native acceptance.** This proposal does not authorize a game launch, saved-profile mutation, new public contract, default-branch change, or closure. Its purpose is to define a safe implementation path at the exact existing owner boundaries. Review and acceptance by the Gateway and GameMod owners are mandatory before writing consumer code.

## Source-of-truth conflict

The Gateway's current accepted process adapter (`process_native_config.rs`, blob `a7b5295835b221f4df5c14eeecc7910e66d7976c`) binds a numeric approved `LaunchProfileId` to a **fixed** exact `user_data_root`, executable and SHA-256 image; it expands `{user_data}` when the native deployment is validated. `NativeProfile::claim_user_data()` admits only an empty private directory or its exact existing process-owner marker. The process profile catalog independently records a numeric `UserDataConfig.namespace_id` (`service_process_lifecycle_config.rs`, blob `c39f0f14ddcb9568f1f1d720207e702906a595ba`).

Save-profile provisioning uses a **different** `UserDataIdentity(u64)` and `FilesystemUserDataPort` with an allocated `run-<identity>` child and provenance marker. Its `LaunchProfileBindingPort` currently returns gateway-local string `automation-disposable-v1` (`save_profile/types.rs`, blob `d4aead6691d4c7beda4b8da90198881677123909`); that is not the numeric native `LaunchProfileId` or GameMod's opaque `SaveSlotId`. The attached runtime (`service_runtime.rs` blob `f5265b7c6408a2a3dc7a725e7a1280f21bfae873`) sets `SaveProfileActiveRun::UNCONFIGURED`, while `service_save_profile.rs` injects unavailable durable allocation/launch dependencies; these refusals are **correct**. Draft Gateway PR #152 head `7252e4f3140e5baddb181ce8ffb97ee5f16c262b` adds **positive-only** rejection for retained owners and unsettled process operations; absence is never proof of idle.

## Decision needed: approved root ownership

Two owner-compatible architectural alternatives exist; **neither is currently approved**:

1. **Finite preapproved disposable-profile pool (recommended initial investigation):** the operator predeclares one exact empty/private native `user_data_root` per numeric approved process profile and a bounded finite pool. A Gateway-owned durable allocator atomically reserves a free pool entry for one `UserDataIdentity`, process `LaunchProfileId`, immutable executable/image digest, namespace, instance and operation. A single identity's reserved entry is never reassigned while a process, pending operation, unknown outcome, unverified marker or active game run might still exist. The save-profile filesystem adapter would require a compatible exact-root/provenance contract; its existing `run-<id>` child path **cannot** silently be treated as the approved native root. The process-native marker and save-profile provenance must be reconciled under a reviewed dual-owner rule, not copied or bypassed. The fixed pool is explicitly capacity-limited, not general arbitrary provisioning.
2. **Versioned server-owned dynamic-root process variant:** accept a new internal owner ABI allowing the Gateway (not a request or workflow) to allocate a fresh canonical root and then bind it to one approved numeric process profile. Add explicit `profile_generation`, authority epoch, immutable approved executable identity, and a durable root binding. Do not interpolate paths into user-configured commands, accept caller roots, follow symlinks, or reuse a path after uncertain create/launch. This changes the ADR 0037 process adapter's fixed-root invariant and requires a new owner decision, compatibility classification and native regression evidence before use. It is not a “small adapter fix.”

**No data migration or unsafe fallback** is permitted. All existing valued and unrelated profiles remain untouched, and the legacy `automation-disposable-v1` string remains a gateway-local synthetic binding until an owner accepts a replacement. If no compatible native-root model is approved, production save-profile mutation continues refusing with `unavailable`.

## Proposed internal durable record

A server-issued allocation record must contain distinct, validated fields, never conflated by numeric equality or string conversion:

| Namespace | Required authoritative origin |
| --- | --- |
| `instance_id`, `session_id`, `lease_id`, `lease_epoch`, caller | authenticated Gateway instance/lease gate; no request override |
| `save_profile_operation_id`, `UserDataIdentity`, allocation-generation | Gateway durable save-profile allocator, persisted before filesystem effects |
| `LaunchProfileId`, approved image/install/executable, user-data namespace | Gateway validated process catalog, not the saved-profile fixed string |
| native canonical root + owner marker and content digest | Gateway preapproved native-root pool or explicitly accepted dynamic-root adapter |
| process generation, authority epoch, retained process/birth identity | Gateway native lifecycle coordinator and owned generation registry |
| `SaveSlotId`, selected-slot baseline and readback revision | actual GameMod host-thread source only, never `CurrentProfileId` integer by assumption |

Suggested transitions (source design only): **Unreserved → Reserved → AllocationIntentDurable → RootCreatedAndVerified → NativeLaunchIntentDurable → OwnerConfirmed → HostSlotReadbackConfirmed → Settled**, with **Unknown/Blocked** substates on every ambiguous external effect. Every write precedes its own effect with an idempotent journal intent. Filesystem, native process and host saves cannot share one ACID transaction, so recovery reconciles the *same* identity and the exact immutable root before deciding; it must not substitute another candidate. Deallocation/reuse requires positive stop confirmation, GameMod idle/readback evidence and preservation proof; otherwise permanently quarantine/retain for operator action.

## Never infer negative idle from absence

Admission for save mutation needs an affirmative **Ready** native owner witness at the exact instance/lease/authority epoch and process generation, plus an independently authoritative GameMod no-active-run witness at the same host incarnation and a reconciled durable operation ledger. *All* of these are required:

- current authenticated lease and nonexpired admission, same instance/caller/session/epoch;
- explicit native Ready status with verified executable/image/birth identity and root ownership, not merely a configured profile ID;
- no active or uncertain process for this allocation, no retained ownership and no unsettled lifecycle operation, verified by a fenced **affirmative owner query**, not the absence of an optional map entry;
- no active game run, exact host/save-slot identity and baseline, and an affirmative host-thread readback before and after any mutation;
- durable allocation/forwarding journals online, reconciled, uniquely owned and cryptographically or mechanically tied to the same root and operation.

An unavailable owner, stale grant, lost reply, restarted generation, missing live-host observation, unrelated profile, or detected positive retained-owner/operation row always causes a refusal. Current PR #152's exclusions remain a necessary safety layer but are not a negative-idle writer. `SaveProfileActiveRun::governed = Some(false)` must never be synthesized by tests, absence of records or `profiles()` enumeration in production.

## Concrete tests to qualify the decision

**Deterministic source:** duplicate operation and cross-instance isolation; stale lease/generation; restart between intent and root creation; crash after creation before reply; same-directory/symlink/canonical alias collision; unknown or foreign provenance; preexisting native marker versus save-profile provenance conflict; two coordinators contend for one pool entry; exhaustion refuses without replacing an identity; out-of-scope/valued profiles untouched; positive retained/unsettled owner blocks even when a synthetic idle port claims false; missing owner never grants negative-idle; native `LaunchProfileId` and `SaveSlotId` never auto-coerce.

**Owner/consumer integration:** real GameMod host-thread list/current/select/readback with opaque SaveSlotId, proof that a rejected operation leaves both selection and original saves unchanged, exact baseline readback after lost response/restart, same operation id replays without second effect, compatible protocol/MCP/Harness/Studio consumer pins and contract digests. Do not manufacture a GameMod oracle from fake or `SaveManager.CurrentProfileId` numeric.

**Actual native qualification:** separately authorized disposable Linux/systemd process with exact approved image, user manager and root; launched/ready/stop/fault/recovery/descendant boundaries; no foreign/valued profile modifications; Windows game host and real GameMod bridge acceptance if applicable; provider/deployment/24-hour soak separately evidenced. No such runs were performed here.

## Ordered implementation handoff

Gateway #50 owner first chooses and records a compatible native-root and generation/lease binding; Gateway #51 consumes the decision in a durable allocation bridge with fail-closed boot validation; GameMod #78 owns and accepts the actual host-thread SaveSlotId baseline and readback; Protocol/MCP/Harness/Studio adopt only accepted versioned contracts in producer-to-consumer order; complete full local Cargo/.NET/Node gates and exact-head hosted CI, then separately authorized native acceptance before any merge/issue closure. **The current production refusals must remain until every precondition exists.**

This is a source-derived engineering proposal, not an assertion that the above decisions, code, signing, native resources or acceptance have occurred.
