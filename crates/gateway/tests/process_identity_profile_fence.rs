// SPDX-License-Identifier: MIT

//! Direct coverage for the `matches_profile` ownership fence.
//!
//! Every clause of `ProcessIdentity::matches_profile` and
//! `ProcessDescendantIdentity::matches_profile` is asserted directly here rather than only through
//! a supervisor or lifecycle path. The existing suites exercise this predicate as a filter inside
//! larger flows, which leaves the individual clauses unpinned: a clause can be dropped and the
//! surrounding flow still refuses for some other reason, so the suite stays green while the fence
//! itself is weaker.

use sts2_gateway::{
    ExecutableIdentity, InstanceId, LaunchProfile, LaunchProfileId, ProcessDescendantIdentity,
    ProcessHandle, ProcessIdentity, ProcessPolicy, UserDataConfig,
};

const OWNER: InstanceId = InstanceId::new(9);

fn resolved_profile() -> Result<LaunchProfile, String> {
    let executable =
        ExecutableIdentity::try_new(31, 32, 33).map_err(|error| format!("{error:?}"))?;
    let user_data = UserDataConfig::try_new(34).map_err(|error| format!("{error:?}"))?;
    let policy = ProcessPolicy::try_new(2, 1_000, 1_000).map_err(|error| format!("{error:?}"))?;
    LaunchProfile::try_new(LaunchProfileId::new(1), executable, user_data, policy)
        .map_err(|error| format!("{error:?}"))
}

fn matching_executable() -> ExecutableIdentity {
    ExecutableIdentity::new(31, 32, 33)
}

fn identity() -> ProcessIdentity {
    ProcessIdentity::new(
        OWNER,
        ProcessHandle::new(7),
        4_242,
        8_181,
        matching_executable(),
        UserDataConfig::new(34),
    )
}

fn descendant() -> ProcessDescendantIdentity {
    ProcessDescendantIdentity::new(
        OWNER,
        4_243,
        8_182,
        matching_executable(),
        UserDataConfig::new(34),
    )
}

#[test]
fn an_exactly_matching_identity_is_accepted() -> Result<(), String> {
    // The positive control: without it, a fence that refused everything would satisfy every
    // negative below.
    let profile = resolved_profile()?;
    assert!(identity().matches_profile(OWNER, profile));
    assert!(descendant().matches_profile(OWNER, profile));
    Ok(())
}

#[test]
fn an_identity_is_refused_for_a_foreign_instance() -> Result<(), String> {
    // The tenant fence. Without the instance clause a process owned by one instance would be
    // adopted by another, which is a cross-tenant ownership error rather than a cleanup miss.
    let profile = resolved_profile()?;
    assert!(!identity().matches_profile(InstanceId::new(10), profile));
    assert!(!descendant().matches_profile(InstanceId::new(10), profile));
    Ok(())
}

#[test]
fn an_identity_is_refused_when_the_birth_id_is_absent() -> Result<(), String> {
    // The PID-reuse fence. A recycled PID can carry a live-looking handle and pid while its birth
    // id is unknown, so a missing birth id must never read as "this is still our process".
    let profile = resolved_profile()?;
    assert!(
        !ProcessIdentity::new(
            OWNER,
            ProcessHandle::new(7),
            4_242,
            0,
            matching_executable(),
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    assert!(
        !ProcessDescendantIdentity::new(
            OWNER,
            4_243,
            0,
            matching_executable(),
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    Ok(())
}

#[test]
fn an_identity_is_refused_when_the_pid_is_absent() -> Result<(), String> {
    let profile = resolved_profile()?;
    assert!(
        !ProcessIdentity::new(
            OWNER,
            ProcessHandle::new(7),
            0,
            8_181,
            matching_executable(),
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    assert!(
        !ProcessDescendantIdentity::new(
            OWNER,
            0,
            8_182,
            matching_executable(),
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    Ok(())
}

#[test]
fn an_identity_is_refused_when_the_process_handle_is_absent() -> Result<(), String> {
    let profile = resolved_profile()?;
    assert!(
        !ProcessIdentity::new(
            OWNER,
            ProcessHandle::new(0),
            4_242,
            8_181,
            matching_executable(),
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    Ok(())
}

#[test]
fn an_identity_is_refused_for_a_foreign_executable_image() -> Result<(), String> {
    let profile = resolved_profile()?;
    let foreign = ExecutableIdentity::new(91, 92, 93);
    assert!(
        !ProcessIdentity::new(
            OWNER,
            ProcessHandle::new(7),
            4_242,
            8_181,
            foreign,
            UserDataConfig::new(34),
        )
        .matches_profile(OWNER, profile)
    );
    assert!(
        !ProcessDescendantIdentity::new(OWNER, 4_243, 8_182, foreign, UserDataConfig::new(34),)
            .matches_profile(OWNER, profile)
    );
    Ok(())
}

#[test]
fn an_identity_is_refused_for_a_foreign_user_data_namespace() -> Result<(), String> {
    // The namespace fence: a process running under another instance's isolated user directory is
    // not this profile's process even when its image matches.
    let profile = resolved_profile()?;
    let foreign = UserDataConfig::new(99);
    assert!(
        !ProcessIdentity::new(
            OWNER,
            ProcessHandle::new(7),
            4_242,
            8_181,
            matching_executable(),
            foreign,
        )
        .matches_profile(OWNER, profile)
    );
    assert!(
        !ProcessDescendantIdentity::new(OWNER, 4_243, 8_182, matching_executable(), foreign,)
            .matches_profile(OWNER, profile)
    );
    Ok(())
}

#[test]
fn a_legacy_identity_without_a_profile_is_never_accepted() -> Result<(), String> {
    // `ProcessIdentity::legacy` records no executable and no user-data directory, so it can never
    // satisfy the fence. Attaching one anyway would adopt a process whose launch profile is
    // unknown.
    let profile = resolved_profile()?;
    assert!(!ProcessIdentity::legacy(ProcessHandle::new(7)).matches_profile(OWNER, profile));
    Ok(())
}
