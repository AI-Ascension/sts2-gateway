// SPDX-License-Identifier: MIT

use super::{AuthFailure, AuthPolicy, AuthScope};

#[test]
fn current_token_requires_scope_and_expiry() -> Result<(), String> {
    let policy = AuthPolicy::test_with_previous("current", Some(100), None, "read,mutate")?;
    assert_eq!(
        policy.authorize_at(Some("Bearer current"), AuthScope::Read, 99),
        Ok(())
    );
    assert_eq!(
        policy.authorize_at(Some("Bearer current"), AuthScope::Control, 99),
        Err(AuthFailure::Scope)
    );
    assert_eq!(
        policy.authorize_at(Some("Bearer current"), AuthScope::Read, 100),
        Err(AuthFailure::Expired)
    );
    Ok(())
}

#[test]
fn previous_token_is_accepted_during_rotation_until_expiry() -> Result<(), String> {
    let policy = AuthPolicy::test_with_previous(
        "current",
        Some(200),
        Some(("previous", Some(100))),
        "read",
    )?;
    assert_eq!(
        policy.authorize_at(Some("Bearer previous"), AuthScope::Read, 99),
        Ok(())
    );
    assert_eq!(
        policy.authorize_at(Some("Bearer previous"), AuthScope::Read, 100),
        Err(AuthFailure::Expired)
    );
    assert_eq!(
        policy.authorize_at(Some("Bearer missing"), AuthScope::Read, 99),
        Err(AuthFailure::Invalid)
    );
    Ok(())
}
