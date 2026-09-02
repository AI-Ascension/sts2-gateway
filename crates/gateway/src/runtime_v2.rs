// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::runtime_v2_artifact::{
    RUNTIME_V2_ACTION_ID, RUNTIME_V2_ARTIFACT, RUNTIME_V2_EFFECT_KIND, RUNTIME_V2_GENERATOR,
    RUNTIME_V2_MAX_GENERATION, RUNTIME_V2_MAX_TURN_INDEX, RUNTIME_V2_PROTOCOL_VERSION,
    RUNTIME_V2_SCHEMA_DIGEST, RUNTIME_V2_SCHEMA_SOURCE,
};

include!("runtime_v2/contract_types.rs");
include!("runtime_v2/message.rs");
include!("runtime_v2/identity.rs");
include!("runtime_v2/ledger.rs");
include!("runtime_v2/ledger_support.rs");
include!("runtime_v2/helpers.rs");
