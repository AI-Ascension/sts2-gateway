// SPDX-License-Identifier: MIT

use std::{collections::BTreeMap, fmt};

use serde::Deserialize;

/// Serde helper used by the wire contract. A missing member is invalid even when the
/// member's type is an option; explicit `null` remains a valid contract value.
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

include!("seeded_run/context.rs");
include!("seeded_run/types.rs");
include!("seeded_run/message.rs");
include!("seeded_run/validation.rs");

mod ledger;

pub use ledger::{
    SeededRunBinding, SeededRunForwardRequest, SeededRunForwardingPort, SeededRunLedger,
    SeededRunLedgerConfig, SeededRunLedgerError, SeededRunPersistedOperation,
    SeededRunPersistedState, SeededRunReceiptRequest, SeededRunTransportFault,
};
