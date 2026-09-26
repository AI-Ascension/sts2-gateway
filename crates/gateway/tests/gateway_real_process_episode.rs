// SPDX-License-Identifier: MIT

//! Real-process evidence for the repeated-episode lease profile (ADR 0033).
//!
//! `crates/gateway/src/bin/runtime_support/service_episode_consecutive_tests.rs`
//! drives consecutive episodes through `RuntimeService::handle_request`, in the
//! test process. This suite instead spawns the built `sts2-gateway-runtime`
//! binary with `std::process::Command::new` and drives it over real loopback
//! sockets, so every decision asserted here is produced by the *served* gateway
//! process rather than by a library call.
//!
//! The claimed evidence class is exactly that: the served gateway process makes
//! the decisions. This is not a production-mode claim, and no native game,
//! deployment, or 24-hour soak is claimed. The downstream soak remains
//! `ascension-watchdog#58`.

// Test code is allowed to panic: a failed assertion is the expected failure mode.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

// Shared with `gateway_refusal_record`, which reads the served process's
// standard error. This suite only surfaces stderr in its own failure messages,
// so those accessors are unused here.
#[allow(dead_code)]
#[path = "gateway_real_process_episode/gateway.rs"]
mod gateway;
#[path = "gateway_real_process_episode/host.rs"]
mod host;
#[path = "gateway_real_process_episode/host_crypto.rs"]
mod host_crypto;
#[path = "gateway_real_process_episode/wire.rs"]
mod wire;

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use serde_json::{Value, json};
use sts2_gateway::{RUNTIME_V3_SCHEMA_DIGEST, sha256_hex};

use self::gateway::{
    CATALOG_DIGEST, DEPLOYMENT, EPISODE_ONE_OPERATION, GatewayProcess, INCARNATION, INSTANCE,
    RENEWAL_INTERVAL_SECONDS, STATE_ID, TTL_SECONDS, TempStore,
};
use self::host::HostFake;
use self::wire::{
    EPISODE_PROFILE, acquire, call, error_code, host_fence, instance_release, original_context,
    release_set,
};

/// Canonical RCJ-1 episode-one action. The bytes are exactly what
/// `serde_json::to_vec` emits for the same value, which is what RCJ-1 requires.
const EPISODE_ONE_ACTION: &[u8] =
    br#"{"action":{"kind":"proceed"},"action_id":"episode-one-action"}"#;

fn episode_one_operation(lease: &Value) -> Value {
    let digest = sha256_hex(EPISODE_ONE_ACTION);
    json!({
        "operation_id": EPISODE_ONE_OPERATION,
        "payload_digest": digest,
        "original_context": original_context(lease),
        "expected_boundary": {
            "state_id": STATE_ID,
            "generation": 0,
            "catalog_digest": CATALOG_DIGEST,
        },
        "action": {
            "schema_digest": RUNTIME_V3_SCHEMA_DIGEST,
            "canonical_json_b64": STANDARD_NO_PAD.encode(EPISODE_ONE_ACTION),
            "payload_digest": digest,
        },
    })
}

/// The three-key operation *reference* that dispatch, lookup, and reconcile
/// accept. Only the intent route takes the full five-key operation payload.
fn episode_one_ref(lease: &Value) -> Value {
    json!({
        "operation_id": EPISODE_ONE_OPERATION,
        "payload_digest": sha256_hex(EPISODE_ONE_ACTION),
        "original_context": original_context(lease),
    })
}

/// Bootstrap, host-fence, then acquire one episode lease from the served
/// process. The boot and the fence are durable identities of the deployment,
/// so they are returned for the next episode to present again; the lease is the
/// per-episode identity.
fn begin_episode(address: &str) -> Result<(Value, Value, Value), String> {
    let (status, bootstrap) = call(
        address,
        "/v1/recovery/bootstrap",
        "bootstrap_request",
        "bootstrap",
        json!({
            "deployment_id": DEPLOYMENT,
            "instance_id": INSTANCE,
            "instance_incarnation": INCARNATION,
            "release": release_set(),
            "lease_policy": {
                "ttl_seconds": TTL_SECONDS,
                "renewal_interval_seconds": RENEWAL_INTERVAL_SECONDS,
            },
        }),
    )?;
    assert_eq!(status, 200, "bootstrap failed: {bootstrap}");
    let mut boot = bootstrap["payload"]["boot"].clone();

    let (status, fenced) = host_fence(address, &boot)?;
    assert_eq!(status, 200, "host fence failed: {fenced}");
    let fence = fenced["payload"]["fence"].clone();
    // The durable transition is authoritative: the committed boot is READY even
    // though the bootstrap response reported FENCE_REQUIRED.
    boot["state"] = json!("READY");

    let (status, allocated) = acquire(address, &boot, &fence)?;
    assert_eq!(status, 200, "episode acquisition failed: {allocated}");
    Ok((boot, fence, allocated["payload"]["lease"].clone()))
}

fn episode_one_intent(address: &str, lease: &Value) -> Result<(u16, Value), String> {
    call(
        address,
        "/v1/recovery/operation/intent",
        "operation_intent_request",
        "operation_submit",
        json!({"lease": lease, "operation": episode_one_operation(lease)}),
    )
}

#[test]
fn consecutive_episodes_on_a_spawned_gateway_land_on_distinct_leases() -> Result<(), String> {
    let host = HostFake::start()?;
    let store = TempStore::new("consecutive");
    let mut gateway = GatewayProcess::spawn(host.address(), store.path())?;
    gateway.await_ready()?;
    let address = gateway.address().to_owned();

    let (boot, fence, lease_one) = begin_episode(&address)?;

    // Episode one records a durable receipt for its own lease.
    let (status, recorded) = episode_one_intent(&address, &lease_one)?;
    assert_eq!(status, 200, "episode-one intent failed: {recorded}");
    assert_eq!(recorded["payload"]["result"]["status"], "INTENT_RECORDED");

    // A profiled release completes episode one and reopens admission.
    let (status, released) = instance_release(&address, &lease_one, Some(EPISODE_PROFILE))?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    assert_eq!(released["status"], "released");
    assert_eq!(
        released["episode_profile"]["released_epoch"],
        lease_one["lease_epoch"]
    );
    assert_eq!(
        released["episode_profile"]["capability"],
        "sts2-gateway/repeated-episode-lease-v1"
    );
    assert_eq!(
        released["episode_profile"]["schema_digest"],
        "f3a04bab61ce4898eda0fa88cb546441493e49eef19b1e5e0841a3b4ef7c4331"
    );

    // Episode two must land on the same boot authority with a new identity.
    let (status, allocated) = acquire(&address, &boot, &fence)?;
    assert_eq!(status, 200, "episode two was refused: {allocated}");
    let lease_two = allocated["payload"]["lease"].clone();
    assert_eq!(
        lease_one["boot_id"], lease_two["boot_id"],
        "consecutive episodes must share one boot authority"
    );
    assert_ne!(
        lease_one["lease_id"], lease_two["lease_id"],
        "episode two must not reuse episode one's lease identity"
    );
    assert!(
        lease_two["lease_epoch"].as_u64() > lease_one["lease_epoch"].as_u64(),
        "episode two must land on a strictly higher epoch: {} then {}",
        lease_one["lease_epoch"],
        lease_two["lease_epoch"]
    );

    // Episode two cannot dispatch episode one's receipt: the receipt is bound to
    // the presenting lease context, and the probe is a mutation-bearing route.
    let (status, refused) = call(
        &address,
        "/v1/recovery/operation/dispatch",
        "operation_dispatch_request",
        "operation_submit",
        json!({"lease": lease_two, "operation": episode_one_ref(&lease_one)}),
    )?;
    assert_eq!(
        status, 409,
        "episode two dispatched an episode-one receipt: {refused}"
    );
    assert_eq!(error_code(&refused), "recovery_operation_context_mismatch");

    // Nor can it resolve that receipt by presenting an episode-two context.
    // The store still holds episode one's row, so the lookup reaches the
    // context comparison and the lease identity is what refuses it.
    let (status, mismatch) = call(
        &address,
        "/v1/recovery/operation/lookup",
        "operation_lookup_request",
        "recovery_read",
        json!({
            "operation": episode_one_ref(&lease_two),
            "lookup_scope": "historical_read",
        }),
    )?;
    assert_eq!(
        status, 409,
        "episode one's receipt resolved under episode two: {mismatch}"
    );
    assert_eq!(error_code(&mismatch), "recovery_operation_context_mismatch");

    // Episode one's fenced headers are refused by the live episode-two fence.
    let (status, stale) = instance_release(&address, &lease_one, None)?;
    assert_eq!(
        status, 409,
        "a released lease still fenced mutations: {stale}"
    );
    assert_eq!(error_code(&stale), "lease_fence_rejected");

    // A header-less replay of the completed episode stays fail-closed.
    let (status, legacy) = instance_release(&address, &lease_two, None)?;
    assert_eq!(status, 200, "legacy release failed: {legacy}");
    assert!(
        legacy.get("episode_profile").is_none(),
        "a header-less release must not echo a negotiated profile: {legacy}"
    );
    let (status, refused) = acquire(&address, &boot, &fence)?;
    assert_eq!(
        status, 409,
        "a header-less completion reopened admission: {refused}"
    );
    assert_eq!(error_code(&refused), "lease_context_revoked");

    gateway.terminate()?;
    let observed = host.observed();
    assert_eq!(
        observed,
        vec![
            "host_fence_request",
            "lease_install_request",
            "lease_revoke_request",
            "lease_install_request",
            "lease_revoke_request",
        ],
        "the served gateway must have driven the signed host sideband"
    );
    Ok(())
}

#[test]
fn a_restarted_gateway_process_discards_the_repeated_episode_profile() -> Result<(), String> {
    let host = HostFake::start()?;
    let store = TempStore::new("restart");

    let mut first = GatewayProcess::spawn(host.address(), store.path())?;
    first.await_ready()?;
    let address = first.address().to_owned();
    let (_, _, lease_one) = begin_episode(&address)?;
    let (status, released) = instance_release(&address, &lease_one, Some(EPISODE_PROFILE))?;
    assert_eq!(status, 200, "profiled release failed: {released}");
    let first_pid = first.id();
    first.terminate()?;

    // A fresh process on the same durable store has no gateway-local profile.
    let mut second = GatewayProcess::spawn(host.address(), store.path())?;
    second.await_ready()?;
    assert_ne!(
        second.id(),
        first_pid,
        "the restart must be a distinct process"
    );
    let address = second.address().to_owned();
    let (boot, fence, lease_two) = begin_episode(&address)?;
    assert_ne!(
        boot["boot_id"], lease_one["boot_id"],
        "a restart must rotate the boot authority"
    );
    assert_ne!(lease_two["lease_id"], lease_one["lease_id"]);
    assert!(lease_two["lease_epoch"].as_u64() > lease_one["lease_epoch"].as_u64());

    // The profile is process-local state, so the restarted process reports no
    // witness for a header-less release and stays permanently revoking.
    let (status, legacy) = instance_release(&address, &lease_two, None)?;
    assert_eq!(status, 200, "restarted release failed: {legacy}");
    assert!(
        legacy.get("episode_profile").is_none(),
        "the restarted process echoed a profile it never negotiated: {legacy}"
    );
    let (status, refused) = acquire(&address, &boot, &fence)?;
    assert_eq!(
        status, 409,
        "the restarted process reopened admission without a negotiated profile: {refused}"
    );
    assert_eq!(error_code(&refused), "lease_context_revoked");

    second.terminate()?;
    let observed = host.observed();
    assert_eq!(
        observed,
        vec![
            "host_fence_request",
            "lease_install_request",
            "lease_revoke_request",
            "host_fence_request",
            "lease_install_request",
            "lease_revoke_request",
        ],
        "both processes must have driven the signed host sideband"
    );
    Ok(())
}
