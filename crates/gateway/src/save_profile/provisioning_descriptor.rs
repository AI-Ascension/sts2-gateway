// SPDX-License-Identifier: MIT

use super::types::{
    LAUNCH_PROFILE_CONTRACT, SaveProfileContext, UserDataDescriptor, UserDataIdentity,
    UserDataProvenance,
};

/// Provenance owner recorded for every gateway-owned isolated allocation.
pub const PROVENANCE_OWNER: &str = "gateway";

pub(crate) fn descriptor(
    context: &SaveProfileContext,
    operation_id: &str,
    identity: UserDataIdentity,
) -> UserDataDescriptor {
    UserDataDescriptor {
        identity,
        provenance: UserDataProvenance {
            owner: String::from(PROVENANCE_OWNER),
            instance_id: context.instance_id.clone(),
            operation_id: operation_id.to_owned(),
            contract: LAUNCH_PROFILE_CONTRACT.to_owned(),
        },
        baseline: None,
    }
}
