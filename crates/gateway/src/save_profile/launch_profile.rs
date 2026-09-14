// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use super::types::{
    LAUNCH_PROFILE_ID, LaunchProfileBinding, LaunchProfileBindingError, LaunchProfileBindingPort,
    UserDataIdentity,
};

/// Deterministic launch-profile adapter used by source/component tests.
///
/// It returns only the single approved `automation-disposable-v1` binding for the requested
/// opaque identity, or an injected refusal. It never reads environment, paths, configuration,
/// or host state.
#[derive(Clone, Debug, Default)]
pub struct InMemoryLaunchProfileBindingPort {
    refusals: BTreeSet<UserDataIdentity>,
    calls: Vec<UserDataIdentity>,
}

impl InMemoryLaunchProfileBindingPort {
    /// Refuse every future binding call for this identity.
    pub fn refuse(&mut self, identity: UserDataIdentity) {
        self.refusals.insert(identity);
    }

    /// Identities passed to `bind_disposable`, in call order.
    pub fn calls(&self) -> &[UserDataIdentity] {
        &self.calls
    }
}

impl LaunchProfileBindingPort for InMemoryLaunchProfileBindingPort {
    fn bind_disposable(
        &mut self,
        user_data: UserDataIdentity,
    ) -> Result<LaunchProfileBinding, LaunchProfileBindingError> {
        self.calls.push(user_data);
        if self.refusals.contains(&user_data) {
            return Err(LaunchProfileBindingError::UserData);
        }
        LaunchProfileBinding::try_new(LAUNCH_PROFILE_ID, user_data)
    }
}

impl<T: LaunchProfileBindingPort + ?Sized> LaunchProfileBindingPort for Box<T> {
    fn bind_disposable(
        &mut self,
        user_data: UserDataIdentity,
    ) -> Result<LaunchProfileBinding, LaunchProfileBindingError> {
        (**self).bind_disposable(user_data)
    }
}
