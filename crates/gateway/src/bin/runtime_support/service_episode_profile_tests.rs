// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use sts2_gateway::{RecoveryBootContext, RecoveryBootState, RecoveryReleaseSet, sha256_hex};

use super::super::test_support::test_service;
use super::super::{RuntimeService, json_error};
use super::{
    EPISODE_PROFILE_CAPABILITY, EPISODE_PROFILE_HEADER, EPISODE_PROFILE_NAME,
    EPISODE_PROFILE_SCHEMA_DIGEST, EpisodeProfile,
};

/// The canonical descriptor recorded in the module doc comment. The digest is
/// asserted against the raw bytes of this literal so a reviewer can reproduce
/// the constant without trusting the code that carries it.
const CANONICAL_DESCRIPTOR: &str = concat!(
    r#"{"admission":"explicit-header-negotiated","#,
    r#""capability":"sts2-gateway/repeated-episode-lease-v1","#,
    r#""header":"x-sts2-episode-profile","#,
    r#""profile":"repeated-episode-lease-v1","#,
    r#""reopen":"completed-episode-release","#,
    r#""scope":"deployment","version":1}"#,
);

fn boot(boot_id: &str, incarnation: &str, authority_generation: u64) -> RecoveryBootContext {
    RecoveryBootContext {
        deployment_id: String::from("deployment-1"),
        instance_id: String::from("instance-1"),
        instance_incarnation: incarnation.to_owned(),
        boot_id: boot_id.to_owned(),
        authority_generation,
        release: RecoveryReleaseSet::unconfigured(),
        created_at_millis: 1,
        state: RecoveryBootState::Ready,
    }
}

fn negotiate(header: Option<&str>) -> EpisodeProfile {
    EpisodeProfile::negotiate(header, &boot("boot-1", "incarnation-1", 4))
        .expect("negotiation must succeed")
        .expect("a requested profile must bind")
}

#[test]
fn schema_digest_is_the_documented_canonical_descriptor() {
    assert_eq!(
        sha256_hex(CANONICAL_DESCRIPTOR.as_bytes()),
        EPISODE_PROFILE_SCHEMA_DIGEST
    );
    assert_eq!(EPISODE_PROFILE_NAME, "repeated-episode-lease-v1");
    assert_eq!(EPISODE_PROFILE_HEADER, "x-sts2-episode-profile");
    assert_eq!(
        EPISODE_PROFILE_CAPABILITY,
        "sts2-gateway/repeated-episode-lease-v1"
    );
}

#[test]
fn negotiation_is_explicit_and_rejects_unknown_profiles() {
    assert!(
        EpisodeProfile::negotiate(None, &boot("boot-1", "incarnation-1", 4))
            .expect("absent header is not an error")
            .is_none(),
        "the single-episode default must survive an absent header"
    );
    for unsupported in ["", "repeated-episode-lease-v2", "REPEATED-EPISODE-LEASE-V1"] {
        assert_eq!(
            EpisodeProfile::negotiate(Some(unsupported), &boot("boot-1", "incarnation-1", 4)),
            Err(()),
            "unsupported profile {unsupported:?} must fail closed"
        );
    }
}

#[test]
fn completed_epoch_floor_admits_only_strictly_higher_epochs() {
    let mut profile = negotiate(Some(EPISODE_PROFILE_NAME));
    assert!(
        profile.admits_epoch(1),
        "a fresh profile must admit any epoch"
    );
    profile.record_completed(3);
    assert!(!profile.admits_epoch(1));
    assert!(
        !profile.admits_epoch(3),
        "the completed epoch must not reopen"
    );
    assert!(profile.admits_epoch(4));
    profile.record_completed(2);
    assert_eq!(
        profile.released_epoch,
        Some(3),
        "the floor must not regress"
    );
    profile.record_completed(9);
    assert!(profile.admits_epoch(10));
    assert!(!profile.admits_epoch(9));
}

#[test]
fn witness_reports_the_negotiated_capability_and_no_host_material() -> Result<(), String> {
    let mut profile = negotiate(Some(EPISODE_PROFILE_NAME));
    let witness = profile.witness();
    assert_eq!(
        witness,
        serde_json::json!({
            "profile": EPISODE_PROFILE_NAME,
            "capability": EPISODE_PROFILE_CAPABILITY,
            "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST,
            "scope": "deployment",
            "admission": "explicit-header-negotiated",
            "reopen": "completed-episode-release",
            "version": 1,
            "released_epoch": serde_json::Value::Null,
        })
    );
    profile.record_completed(7);
    let witness = profile.witness();
    assert_eq!(witness["released_epoch"], 7);
    let rendered = serde_json::to_string(&witness).map_err(|error| error.to_string())?;
    for leaked in ["boot-1", "incarnation-1", "deployment-1", "proof", "secret"] {
        assert!(
            !rendered.contains(leaked),
            "witness must not carry host material: {rendered}"
        );
    }
    Ok(())
}

#[test]
fn a_rotated_boot_authority_does_not_inherit_the_profile() {
    let mut profile = negotiate(Some(EPISODE_PROFILE_NAME));
    profile.record_completed(1);
    let same = boot("boot-1", "incarnation-1", 4);
    assert!(profile.matches_boot(&same));
    for other in [
        boot("boot-2", "incarnation-1", 4),
        boot("boot-1", "incarnation-2", 4),
        boot("boot-1", "incarnation-1", 5),
    ] {
        assert!(
            !profile.matches_boot(&other),
            "a different boot/incarnation/generation must not bind: {other:?}"
        );
    }
}

#[test]
fn profiled_release_requires_an_available_boot_authority() -> Result<(), String> {
    let service = test_service()?;
    assert!(service.recovery_boot.is_none(), "fixture has no boot");
    assert_eq!(service.negotiate_episode_profile(None), Ok(None));
    assert_eq!(
        service.negotiate_episode_profile(Some(EPISODE_PROFILE_NAME)),
        Err((503, json_error("episode_profile_boot_required"))),
        "a requested profile must not silently degrade to the default"
    );
    assert_eq!(
        service.negotiate_episode_profile(Some("repeated-episode-lease-v2")),
        Err((503, json_error("episode_profile_boot_required"))),
        "the missing boot is reported before the unsupported value"
    );
    Ok(())
}

#[test]
fn episode_admission_refusal_ignores_an_absent_profile() -> Result<(), String> {
    let service: RuntimeService = test_service()?;
    assert!(service.episode_profile.is_none());
    assert!(
        !service.episode_admission_refused(&boot("boot-1", "incarnation-1", 4), 1),
        "the legacy default must not refuse any epoch locally"
    );
    assert!(service.episode_profile_witness().is_none());
    Ok(())
}

struct Bound {
    service: RuntimeService,
    boot: RecoveryBootContext,
}

fn bound_service() -> Result<Bound, String> {
    let mut service = test_service()?;
    let boot = boot("boot-1", "incarnation-1", 4);
    service.recovery_boot = Some(boot.clone());
    service.episode_profile = Some(negotiate(Some(EPISODE_PROFILE_NAME)));
    Ok(Bound { service, boot })
}

#[test]
fn admission_refuses_a_reused_or_earlier_epoch_of_the_same_boot() -> Result<(), String> {
    let mut bound = bound_service()?;
    assert!(!bound.service.episode_admission_refused(&bound.boot, 1));
    bound
        .service
        .episode_profile
        .as_mut()
        .ok_or_else(|| String::from("profile missing"))?
        .record_completed(5);
    assert!(bound.service.episode_admission_refused(&bound.boot, 5));
    assert!(bound.service.episode_admission_refused(&bound.boot, 4));
    assert!(!bound.service.episode_admission_refused(&bound.boot, 6));
    let rotated = boot("boot-2", "incarnation-1", 5);
    assert!(
        !bound.service.episode_admission_refused(&rotated, 1),
        "a rotated boot is a fresh admission context"
    );
    Ok(())
}

#[test]
fn completion_reopens_admission_and_drops_the_released_lease_identity() -> Result<(), String> {
    let mut bound = bound_service()?;
    bound.service.lease_revoked = true;
    bound.service.episode_profile = None;
    bound.service.commit_completed_episode(3);
    assert!(
        bound.service.lease_revoked,
        "completion without a bound profile must stay permanently closed"
    );

    let mut bound = bound_service()?;
    bound.service.lease_revoked = true;
    bound.service.commit_completed_episode(3);
    assert!(
        !bound.service.lease_revoked,
        "a bound completion must reopen exactly one admission context"
    );
    assert_eq!(
        bound
            .service
            .episode_profile
            .as_ref()
            .ok_or_else(|| String::from("profile missing"))?
            .released_epoch,
        Some(3)
    );
    assert!(bound.service.recovery_lease.is_none());
    assert!(bound.service.recovery_host_grant.is_none());
    assert!(bound.service.recovery_lease_deadline.is_none());
    assert!(bound.service.episode_profile_witness().is_some());
    Ok(())
}

#[test]
fn completion_under_a_rotated_boot_keeps_admission_closed() -> Result<(), String> {
    let mut bound = bound_service()?;
    bound.service.recovery_boot = Some(boot("boot-2", "incarnation-1", 5));
    bound.service.lease_revoked = true;
    bound.service.commit_completed_episode(3);
    assert!(
        bound.service.lease_revoked,
        "a profile that no longer binds the boot must not reopen admission"
    );
    Ok(())
}
